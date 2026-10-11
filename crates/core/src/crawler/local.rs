//! Standalone production crawler: the batch executor, a durable web frontier,
//! per-page WARC files, and observations consumed by the Svelte control panel.

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::Utc;
use fs4::FileExt;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    config::{CrawlerConfig, S3Config, UserAgent},
    warc,
    webpage::Html,
};

use super::{
    observer::{Counters, FrontierState, Observer, Snapshot, Warc},
    robot_client::RobotClient,
    CrawlDatum, DatumSink, Domain, Job, JobExecutor, WeightedUrl,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub seeds: Vec<String>,
    pub seeds_file: Option<PathBuf>,
    pub output_path: PathBuf,
    pub status_path: PathBuf,
    pub pages_per_domain: u64,
    pub num_worker_threads: usize,
    pub min_crawl_delay_ms: u64,
    pub timeout_seconds: u64,
    pub user_agent: UserAgent,
    #[serde(default = "default_frontier_capacity")]
    pub max_frontier: usize,
    #[serde(default = "default_revisit_interval")]
    pub revisit_interval_seconds: u64,
}

fn default_frontier_capacity() -> usize {
    10_000
}
fn default_revisit_interval() -> u64 {
    3600
}

impl Config {
    fn crawler_config(&self) -> CrawlerConfig {
        CrawlerConfig {
            num_worker_threads: self.num_worker_threads,
            user_agent: self.user_agent.clone(),
            robots_txt_cache_sec: 3600,
            min_politeness_factor: 0,
            start_politeness_factor: 0,
            min_crawl_delay_ms: self.min_crawl_delay_ms,
            max_crawl_delay_ms: 180_000.max(self.min_crawl_delay_ms),
            max_politeness_factor: 11,
            max_url_slowdown_retry: 3,
            timeout_seconds: self.timeout_seconds,
            // The standalone executor never instantiates the S3 writer.
            s3: S3Config {
                bucket: String::new(),
                folder: String::new(),
                access_key: String::new(),
                secret_key: String::new(),
                endpoint: String::new(),
            },
            router_hosts: Vec::new(),
        }
    }

    fn seed_urls(&self) -> Result<Vec<Url>> {
        let mut seeds = self.seeds.clone();
        if let Some(path) = &self.seeds_file {
            seeds.extend(
                fs::read_to_string(path)
                    .with_context(|| format!("Cannot read seeds file {}", path.display()))?
                    .lines()
                    .map(str::trim)
                    .filter(|line| !line.is_empty() && !line.starts_with('#'))
                    .map(str::to_owned),
            );
        }
        anyhow::ensure!(
            !seeds.is_empty(),
            "At least one public seed URL is required"
        );
        seeds
            .into_iter()
            .map(|seed| {
                let mut url = Url::parse(&seed).with_context(|| format!("Invalid seed {seed}"))?;
                anyhow::ensure!(
                    matches!(url.scheme(), "https" | "http")
                        && !Domain::from(&url).as_str().is_empty(),
                    "Invalid public web seed {seed}"
                );
                anyhow::ensure!(
                    url.username().is_empty() && url.password().is_none(),
                    "Seed credentials are not supported"
                );
                url.set_fragment(None);
                Ok(url)
            })
            .collect()
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Queue {
    pending: VecDeque<Job>,
    active: BTreeMap<String, Job>,
    seen: HashSet<String>,
}

struct Frontier {
    path: PathBuf,
    queue: Mutex<Queue>,
    capacity: usize,
    budget: u64,
    observer: Arc<Observer>,
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&tmp, path)?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)?.sync_all()?;
    }
    Ok(())
}

impl Frontier {
    fn open(path: PathBuf, capacity: usize, budget: u64, observer: Arc<Observer>) -> Result<Self> {
        let mut queue: Queue = if path.exists() {
            serde_json::from_reader(fs::File::open(&path)?)
                .with_context(|| format!("Invalid persisted crawler frontier {}", path.display()))?
        } else {
            Queue::default()
        };
        // A crash or stop can leave an in-flight job. Replaying it may revisit
        // pages but cannot lose the job or overwrite an already committed WARC.
        let active = std::mem::take(&mut queue.active);
        for (_, job) in active {
            queue.pending.push_front(job);
        }
        let frontier = Self {
            path,
            queue: Mutex::new(queue),
            capacity,
            budget,
            observer,
        };
        frontier.persist(&frontier.queue.lock().unwrap())?;
        Ok(frontier)
    }

    fn persist(&self, queue: &Queue) -> Result<()> {
        atomic_write(&self.path, &serde_json::to_vec(queue)?)?;
        self.observer
            .frontier(queue.pending.len(), queue.seen.len(), self.capacity);
        Ok(())
    }

    fn discover(&self, urls: impl IntoIterator<Item = Url>) -> Result<()> {
        let mut queue = self.queue.lock().unwrap();
        let mut changed = false;
        for mut url in urls {
            if queue.pending.len() >= self.capacity {
                break;
            }
            if !matches!(url.scheme(), "https" | "http")
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some_and(|p| p != 80 && p != 443)
            {
                continue;
            }
            url.set_fragment(None);
            let domain = Domain::from(&url);
            if domain.as_str().is_empty() || queue.seen.contains(domain.as_str()) {
                continue;
            }
            queue.seen.insert(domain.as_str().to_owned());
            queue.pending.push_back(Job {
                domain,
                urls: VecDeque::from([WeightedUrl { url, weight: 1.0 }]),
                wandering_urls: self.budget.saturating_sub(1),
            });
            changed = true;
        }
        if changed {
            self.persist(&queue)?;
        }
        Ok(())
    }

    fn claim(&self) -> Result<Option<Job>> {
        let mut queue = self.queue.lock().unwrap();
        let job = queue.pending.pop_front();
        if let Some(job) = &job {
            queue.active.insert(job.domain.as_str().into(), job.clone());
            self.persist(&queue)?;
        }
        Ok(job)
    }

    fn complete(&self, domain: &Domain) -> Result<()> {
        let mut queue = self.queue.lock().unwrap();
        queue.active.remove(domain.as_str());
        self.persist(&queue)
    }

    fn restart_round(&self, seeds: Vec<Url>) -> Result<()> {
        {
            let mut queue = self.queue.lock().unwrap();
            anyhow::ensure!(
                queue.pending.is_empty() && queue.active.is_empty(),
                "Cannot reset active frontier"
            );
            queue.seen.clear();
        }
        self.discover(seeds)
    }
}

struct LocalWarcSink {
    directory: PathBuf,
    observer: Arc<Observer>,
    frontier: Arc<Frontier>,
    fatal_error: Mutex<Option<String>>,
}

impl LocalWarcSink {
    fn commit(&self, datum: &CrawlDatum) -> Result<()> {
        let mut writer = warc::WarcWriter::new();
        writer.write(&warc::WarcRecord {
            request: warc::Request {
                url: datum.url.to_string(),
                date: Some(datum.date),
            },
            response: warc::Response {
                body: datum.body.clone(),
                payload_type: Some(datum.payload_type),
            },
            metadata: warc::Metadata {
                fetch_time_ms: datum.fetch_time_ms,
            },
        })?;
        let bytes = writer.finish()?;
        let filename = format!("{}.warc.gz", uuid::Uuid::new_v4());
        atomic_write(&self.directory.join(&filename), &bytes)?;
        self.observer.saved(
            &Domain::from(&datum.url),
            datum,
            filename,
            bytes.len() as u64,
        );
        if let Ok(html) = Html::parse(&datum.body, datum.url.as_str()) {
            self.frontier.discover(
                html.anchor_links()
                    .into_iter()
                    .map(|link| link.destination)
                    .take(super::MAX_OUTGOING_URLS_PER_PAGE),
            )?;
        }
        Ok(())
    }

    fn check(&self) -> Result<()> {
        if let Some(error) = self.fatal_error.lock().unwrap().as_ref() {
            anyhow::bail!("Crawler storage failed: {error}");
        }
        self.observer.check()
    }
}

impl DatumSink for LocalWarcSink {
    async fn write(&self, datum: CrawlDatum) -> super::Result<()> {
        if let Err(error) = self.commit(&datum) {
            *self.fatal_error.lock().unwrap() = Some(error.to_string());
            return Err(super::Error::Anyhow(error));
        }
        Ok(())
    }
    async fn finish(&self) -> super::Result<()> {
        self.check().map_err(super::Error::Anyhow)
    }
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result?,
            _ = terminate.recv() => {},
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}

pub async fn run(config: Config) -> Result<()> {
    anyhow::ensure!(
        (1..=32).contains(&config.num_worker_threads),
        "num_worker_threads must be 1..32"
    );
    anyhow::ensure!(
        (1..=10_000).contains(&config.pages_per_domain),
        "pages_per_domain must be 1..10000"
    );
    anyhow::ensure!(
        config.min_crawl_delay_ms >= 10_000,
        "Minimum per-domain crawl delay is 10000ms"
    );
    anyhow::ensure!(
        config.timeout_seconds > 0
            && config.max_frontier > 0
            && config.revisit_interval_seconds >= 60,
        "Invalid crawler limits"
    );
    let seeds = config.seed_urls()?;
    if let Some(parent) = config.status_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(config.status_path.with_extension("lock"))?;
    lock.try_lock_exclusive()
        .context("Another local crawler already owns this state directory")?;
    let crawler_config = Arc::new(config.crawler_config());
    let client = RobotClient::new_public(&crawler_config, &seeds).await?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let directory = config.output_path.join(&run_id);
    fs::create_dir_all(&directory)?;
    let now = Utc::now().to_rfc3339();
    let observer = Arc::new(Observer::new(
        config.status_path.clone(),
        Snapshot {
            schema_version: 1,
            run_id,
            pid: std::process::id(),
            started_at: now.clone(),
            updated_at: now,
            finished_at: None,
            phase: "running".into(),
            user_agent: config.user_agent.full.clone(),
            workers: config.num_worker_threads,
            min_crawl_delay_ms: config.min_crawl_delay_ms,
            pages_per_domain: config.pages_per_domain,
            continuous: true,
            frontier: FrontierState::default(),
            counters: Counters::default(),
            sites: Vec::new(),
            recent_events: Vec::new(),
            history: Vec::new(),
            warc: Warc {
                directory: directory.display().to_string(),
                files: 0,
                bytes: 0,
                current_file: None,
            },
        },
    )?);
    let frontier = Arc::new(Frontier::open(
        config.status_path.with_file_name("frontier.json"),
        config.max_frontier,
        config.pages_per_domain,
        observer.clone(),
    )?);
    frontier.discover(seeds.clone())?;
    let sink = Arc::new(LocalWarcSink {
        directory,
        observer: observer.clone(),
        frontier: frontier.clone(),
        fatal_error: Mutex::new(None),
    });
    let mut jobs = tokio::task::JoinSet::new();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    let mut exhausted_at = None;
    let outcome: Result<()> = async {
        loop {
            sink.check()?;
            while jobs.len() < config.num_worker_threads {
                let Some(job) = frontier.claim()? else { break };
                observer.register_site(job.domain.as_str().into(), config.pages_per_domain);
                let domain = job.domain.clone();
                let executor = JobExecutor::new(
                    job.into(),
                    crawler_config.clone(),
                    sink.clone(),
                    client.clone(),
                )
                .with_observer(observer.clone());
                jobs.spawn(async move {
                    executor.run().await;
                    domain
                });
            }
            if jobs.is_empty() {
                let since = exhausted_at.get_or_insert_with(std::time::Instant::now);
                observer.phase("idle");
                if since.elapsed() >= Duration::from_secs(config.revisit_interval_seconds) {
                    frontier.restart_round(seeds.clone())?;
                    exhausted_at = None;
                    observer.phase("running");
                    continue;
                }
            }
            tokio::select! {
                stopped = &mut shutdown => { stopped?; break; }
                _ = heartbeat.tick() => observer.heartbeat(),
                result = jobs.join_next(), if !jobs.is_empty() => {
                    let domain = result.unwrap().context("Crawler worker panicked")?;
                    sink.check()?;
                    frontier.complete(&domain)?;
                }
            }
        }
        Ok(())
    }
    .await;
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    observer.finish(if outcome.is_ok() { "stopped" } else { "failed" });
    outcome?;
    sink.check()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observer(path: PathBuf) -> Arc<Observer> {
        Arc::new(
            Observer::new(
                path,
                Snapshot {
                    schema_version: 1,
                    run_id: "test".into(),
                    pid: std::process::id(),
                    started_at: Utc::now().to_rfc3339(),
                    updated_at: Utc::now().to_rfc3339(),
                    finished_at: None,
                    phase: "running".into(),
                    user_agent: "TestBot".into(),
                    workers: 1,
                    min_crawl_delay_ms: 10_000,
                    pages_per_domain: 5,
                    continuous: true,
                    frontier: FrontierState::default(),
                    counters: Counters::default(),
                    sites: Vec::new(),
                    recent_events: Vec::new(),
                    history: Vec::new(),
                    warc: Warc {
                        directory: String::new(),
                        files: 0,
                        bytes: 0,
                        current_file: None,
                    },
                },
            )
            .unwrap(),
        )
    }

    #[test]
    fn frontier_replays_unfinished_jobs_and_respects_capacity() {
        let dir = tempfile::tempdir().unwrap();
        let telemetry = observer(dir.path().join("status.json"));
        let path = dir.path().join("frontier.json");
        let frontier = Frontier::open(path.clone(), 2, 5, telemetry.clone()).unwrap();
        frontier
            .discover([
                Url::parse("https://example.org/first").unwrap(),
                Url::parse("https://example.org/duplicate-domain").unwrap(),
                Url::parse("https://wikipedia.org/").unwrap(),
                Url::parse("https://rust-lang.org/").unwrap(),
            ])
            .unwrap();
        let first = frontier.claim().unwrap().unwrap();
        let second = frontier.claim().unwrap().unwrap();
        assert!(frontier.claim().unwrap().is_none());
        assert_ne!(first.domain, second.domain);
        assert_eq!(first.wandering_urls, 4);
        frontier.complete(&first.domain).unwrap();
        drop(frontier);
        let restored = Frontier::open(path, 2, 5, telemetry).unwrap();
        assert_eq!(restored.claim().unwrap().unwrap().domain, second.domain);
        assert!(restored.claim().unwrap().is_none());
    }

    #[test]
    fn atomic_warc_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("page.warc.gz");
        let mut writer = warc::WarcWriter::new();
        writer
            .write(&warc::WarcRecord {
                request: warc::Request {
                    url: "https://example.org/article".into(),
                    date: Some(Utc::now()),
                },
                response: warc::Response {
                    body: "<html><body>saved content</body></html>".into(),
                    payload_type: Some(warc::PayloadType::Html),
                },
                metadata: warc::Metadata { fetch_time_ms: 42 },
            })
            .unwrap();
        atomic_write(&path, &writer.finish().unwrap()).unwrap();
        let file = warc::WarcFile::open(&path).unwrap();
        let record = file.records().next().unwrap().unwrap();
        assert_eq!(record.request.url, "https://example.org/article");
        assert_eq!(record.metadata.fetch_time_ms, 42);
        assert!(record.response.body.contains("saved content"));
    }
}
