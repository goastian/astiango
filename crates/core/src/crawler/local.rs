//! Standalone production crawler: the batch executor, a durable web frontier,
//! per-page WARC files, and observations consumed by the Svelte control panel.

use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

use anyhow::{Context, Result};
use chrono::Utc;
use fs4::FileExt;
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    config::{default_max_redirect_hops, CrawlNetworkPolicy, CrawlerConfig, S3Config, UserAgent},
    warc,
    webpage::Html,
};

use super::{
    observer::{Counters, FrontierState, Observer, Snapshot, Warc},
    outcome::{FetchOutcome, JobOutcome, JobStatus},
    robot_client::{validate_seed_urls, RobotClient},
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
    pub state_directory: Option<PathBuf>,
    pub pages_per_domain: u64,
    pub num_worker_threads: usize,
    pub min_crawl_delay_ms: u64,
    pub timeout_seconds: u64,
    pub user_agent: UserAgent,
    #[serde(default)]
    pub network_policy: CrawlNetworkPolicy,
    #[serde(default)]
    pub transport: super::transport::TransportConfig,
    #[serde(default)]
    pub body: super::body::BodyConfig,
    #[serde(default = "default_max_redirect_hops")]
    pub max_redirect_hops: u8,
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
    fn state_directory(&self) -> &Path {
        self.state_directory.as_deref().unwrap_or_else(|| {
            self.status_path
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| Path::new("."))
        })
    }

    fn lock_path(&self) -> PathBuf {
        if self.state_directory.is_some() {
            self.state_directory().join("status.lock")
        } else {
            self.status_path.with_extension("lock")
        }
    }

    fn validate(&self) -> Result<()> {
        self.transport.validate()?;
        self.body.validate()?;
        anyhow::ensure!(
            self.network_policy == CrawlNetworkPolicy::PublicOnly,
            "The local crawler requires network_policy = public_only"
        );
        anyhow::ensure!(
            (1..=32).contains(&self.num_worker_threads),
            "num_worker_threads must be 1..32"
        );
        anyhow::ensure!(
            (1..=10_000).contains(&self.pages_per_domain),
            "pages_per_domain must be 1..10000"
        );
        anyhow::ensure!(
            self.min_crawl_delay_ms >= 10_000,
            "Minimum per-domain crawl delay is 10000ms"
        );
        anyhow::ensure!(
            self.timeout_seconds > 0
                && self.max_frontier > 0
                && self.revisit_interval_seconds >= 60,
            "Invalid crawler limits"
        );
        Ok(())
    }

    fn crawler_config(&self) -> CrawlerConfig {
        CrawlerConfig {
            num_worker_threads: self.num_worker_threads,
            user_agent: self.user_agent.clone(),
            network_policy: self.network_policy,
            transport: self.transport.clone(),
            body: self.body.clone(),
            max_redirect_hops: self.max_redirect_hops,
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
    #[serde(default)]
    delayed: VecDeque<DelayedJob>,
}

#[derive(Serialize, Deserialize)]
struct DelayedJob {
    job: Job,
    not_before: chrono::DateTime<Utc>,
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
        self.observer.frontier(
            queue.pending.len() + queue.delayed.len(),
            queue.seen.len(),
            self.capacity,
        );
        Ok(())
    }

    fn discover(&self, urls: impl IntoIterator<Item = Url>) -> Result<()> {
        let mut queue = self.queue.lock().unwrap();
        let mut changed = false;
        for mut url in urls {
            if queue.pending.len() + queue.delayed.len() + queue.active.len() >= self.capacity {
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
        self.claim_at(Utc::now())
    }

    fn claim_at(&self, now: chrono::DateTime<Utc>) -> Result<Option<Job>> {
        let mut queue = self.queue.lock().unwrap();
        let mut waiting = VecDeque::new();
        while let Some(delayed) = queue.delayed.pop_front() {
            if delayed.not_before <= now {
                queue.pending.push_back(delayed.job);
            } else {
                waiting.push_back(delayed);
            }
        }
        queue.delayed = waiting;
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

    fn resolve(
        &self,
        domain: &Domain,
        outcome: &JobOutcome,
        min_delay_ms: u64,
        now: chrono::DateTime<Utc>,
    ) -> Result<()> {
        match outcome.status {
            JobStatus::Completed => return self.complete(domain),
            JobStatus::StorageFailed => {
                anyhow::bail!(
                    "Crawler storage failed for {}; job remains active",
                    domain.as_str()
                );
            }
            JobStatus::Deferred | JobStatus::Cancelled => {}
        }
        let mut queue = self.queue.lock().unwrap();
        let original = queue
            .active
            .get(domain.as_str())
            .context("Missing active crawler job")?;
        let mut delay_ms = min_delay_ms;
        let mut urls = VecDeque::new();
        let mut seen_urls = HashSet::new();
        for entry in &outcome.urls {
            match &entry.outcome {
                FetchOutcome::Deferred { retry_after_ms, .. } => {
                    delay_ms = delay_ms.max(retry_after_ms.unwrap_or(0));
                }
                FetchOutcome::Cancelled => {}
                _ => continue,
            }
            if seen_urls.insert(entry.url.clone()) {
                let weight = original
                    .urls
                    .iter()
                    .find(|weighted| weighted.url == entry.url)
                    .map_or(0.0, |weighted| weighted.weight);
                urls.push_back(WeightedUrl {
                    url: entry.url.clone(),
                    weight,
                });
            }
        }
        let scheduled: HashSet<_> = original.urls.iter().map(|weighted| &weighted.url).collect();
        let explored = outcome
            .urls
            .iter()
            .filter(|entry| !scheduled.contains(&entry.url))
            .count();
        let wandering_urls = original.wandering_urls.saturating_sub(explored as u64);
        let job = if urls.is_empty() {
            original.clone()
        } else {
            Job {
                domain: domain.clone(),
                urls,
                wandering_urls,
            }
        };
        let not_before = chrono::Duration::from_std(Duration::from_millis(delay_ms))
            .ok()
            .and_then(|delay| now.checked_add_signed(delay))
            .unwrap_or(chrono::DateTime::<Utc>::MAX_UTC);
        queue.active.remove(domain.as_str());
        queue.delayed.push_back(DelayedJob { job, not_before });
        self.persist(&queue)
    }

    fn is_empty(&self) -> bool {
        let queue = self.queue.lock().unwrap();
        queue.pending.is_empty() && queue.active.is_empty() && queue.delayed.is_empty()
    }

    fn restart_round(&self, seeds: Vec<Url>) -> Result<()> {
        {
            let mut queue = self.queue.lock().unwrap();
            anyhow::ensure!(
                queue.pending.is_empty() && queue.active.is_empty() && queue.delayed.is_empty(),
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
    store_warc: fn(&Path, &[u8]) -> Result<()>,
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
        (self.store_warc)(&self.directory.join(&filename), &bytes)?;
        if let Ok(html) = Html::parse(&datum.body, datum.url.as_str()) {
            self.frontier.discover(
                html.anchor_links()
                    .into_iter()
                    .map(|link| link.destination)
                    .take(super::MAX_OUTGOING_URLS_PER_PAGE),
            )?;
        }
        self.observer.saved(
            &Domain::from(&datum.url),
            datum,
            filename,
            bytes.len() as u64,
        );
        Ok(())
    }

    fn check(&self) -> Result<()> {
        if let Some(error) = self.fatal_error.lock().unwrap().as_ref() {
            anyhow::bail!("Crawler storage failed: {error}");
        }
        Ok(())
    }
}

impl DatumSink for LocalWarcSink {
    async fn write(&self, datum: CrawlDatum) -> super::Result<()> {
        self.check().map_err(super::Error::Anyhow)?;
        if let Err(error) = self.commit(&datum) {
            *self.fatal_error.lock().unwrap() = Some(error.to_string());
            return Err(super::Error::Anyhow(error));
        }
        Ok(())
    }
    async fn finish(&self) -> super::Result<()> {
        self.check().map_err(super::Error::Anyhow)
    }
    async fn checkpoint(&self) -> super::Result<()> {
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
    config.validate()?;
    let seeds = config.seed_urls()?;
    fs::create_dir_all(config.state_directory())?;
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(config.lock_path())?;
    lock.try_lock_exclusive()
        .context("Another local crawler already owns this state directory")?;
    let crawler_config = Arc::new(config.crawler_config());
    validate_seed_urls(&crawler_config, &seeds)?;
    let client = RobotClient::new(&crawler_config)?;
    let run_id = uuid::Uuid::new_v4().to_string();
    let directory = config.output_path.join(&run_id);
    fs::create_dir_all(&directory)?;
    let now = Utc::now().to_rfc3339();
    let observer = Arc::new(Observer::new(
        config.status_path.clone(),
        Snapshot {
            schema_version: 1,
            run_id: run_id.clone(),
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
    observer.observe_transport(client.transport().clone());
    observer.observe_body(client.body_reader().clone());
    let frontier = Arc::new(Frontier::open(
        config.state_directory().join("frontier.json"),
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
        store_warc: atomic_write,
    });
    let job_directory = config.state_directory().join("jobs").join(&run_id);
    fs::create_dir_all(&job_directory)?;
    let mut jobs = tokio::task::JoinSet::new();
    let cancellation = Arc::new(AtomicBool::new(false));
    let mut shutting_down = false;
    let mut heartbeat = tokio::time::interval(Duration::from_secs(2));
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    let mut exhausted_at = None;
    let outcome: Result<()> = async {
        loop {
            while !shutting_down && jobs.len() < config.num_worker_threads {
                let Some(job) = frontier.claim()? else { break };
                if jobs.is_empty() {
                    observer.phase("running");
                }
                observer.register_site(job.domain.as_str().into(), config.pages_per_domain);
                let domain = job.domain.clone();
                let executor = JobExecutor::new(
                    job.into(),
                    crawler_config.clone(),
                    sink.clone(),
                    client.clone(),
                )
                .with_observer(observer.clone())
                .with_cancellation(cancellation.clone());
                jobs.spawn(async move { (domain, executor.run().await) });
            }
            if shutting_down && jobs.is_empty() {
                break;
            }
            if jobs.is_empty() {
                observer.phase("idle");
                if frontier.is_empty() {
                    let since = exhausted_at.get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() >= Duration::from_secs(config.revisit_interval_seconds) {
                        frontier.restart_round(seeds.clone())?;
                        exhausted_at = None;
                        continue;
                    }
                } else {
                    exhausted_at = None;
                }
            } else {
                exhausted_at = None;
            }
            tokio::select! {
                stopped = &mut shutdown, if !shutting_down => {
                    stopped?;
                    shutting_down = true;
                    cancellation.store(true, Ordering::Release);
                }
                _ = heartbeat.tick() => observer.heartbeat(),
                result = jobs.join_next(), if !jobs.is_empty() => {
                    let (domain, job_outcome) = result.unwrap().context("Crawler worker panicked")?;
                    persist_job_outcome(&job_directory, &domain, &job_outcome)?;
                    if job_outcome.is_success() {
                        tracing::info!(domain = domain.as_str(), "Crawler job durably resolved");
                    } else {
                        tracing::warn!(domain = domain.as_str(), status = ?job_outcome.status,
                            "Crawler job finished with explicit non-success outcomes");
                    }
                    frontier.resolve(&domain, &job_outcome, config.min_crawl_delay_ms, Utc::now())?;
                    sink.check()?;
                }
            }
        }
        Ok(())
    }
    .await;
    if outcome.is_err() {
        cancellation.store(true, Ordering::Release);
        jobs.abort_all();
        while jobs.join_next().await.is_some() {}
    }
    observer.finish(if outcome.is_ok() { "stopped" } else { "failed" });
    outcome?;
    sink.check()?;
    Ok(())
}

fn persist_job_outcome(directory: &Path, domain: &Domain, outcome: &JobOutcome) -> Result<()> {
    let record = serde_json::json!({
        "schema_version": 1,
        "domain": domain.as_str(),
        "finished_at": Utc::now(),
        "successful": outcome.is_success(),
        "outcome": outcome,
    });
    atomic_write(
        &directory.join(format!("{}.json", uuid::Uuid::new_v4())),
        &serde_json::to_vec(&record)?,
    )
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
    fn deferred_urls_survive_restart_and_cannot_run_before_retry_after() {
        let dir = tempfile::tempdir().unwrap();
        let telemetry = observer(dir.path().join("status.json"));
        let path = dir.path().join("frontier.json");
        let frontier = Frontier::open(path.clone(), 2, 5, telemetry.clone()).unwrap();
        let seed = Url::parse("https://example.org/").unwrap();
        frontier.discover([seed.clone()]).unwrap();
        let job = frontier.claim().unwrap().unwrap();
        let retry = Url::parse("https://example.org/retry").unwrap();
        let mut outcome = JobOutcome::default();
        outcome.record(
            seed.clone(),
            FetchOutcome::DurableSuccess {
                final_url: seed.clone(),
            },
        );
        outcome.record(
            retry.clone(),
            FetchOutcome::Deferred {
                reason: "http_429".into(),
                retry_after_ms: Some(300_000),
            },
        );
        outcome.record(
            Url::parse("https://example.org/missing").unwrap(),
            FetchOutcome::PermanentFailure {
                reason: "http_404".into(),
            },
        );
        let now = Utc::now();
        frontier
            .resolve(&job.domain, &outcome, 10_000, now)
            .unwrap();
        assert!(!frontier.is_empty());
        assert!(frontier
            .claim_at(now + chrono::Duration::seconds(299))
            .unwrap()
            .is_none());
        assert!(frontier.restart_round(vec![seed]).is_err());
        drop(frontier);
        let restored = Frontier::open(path, 2, 5, telemetry).unwrap();
        assert!(restored
            .claim_at(now + chrono::Duration::seconds(299))
            .unwrap()
            .is_none());
        let retried = restored
            .claim_at(now + chrono::Duration::seconds(300))
            .unwrap()
            .unwrap();
        assert_eq!(retried.urls.len(), 1);
        assert_eq!(retried.urls[0].url, retry);
        assert_eq!(retried.wandering_urls, 2);
    }

    #[test]
    fn deferred_seed_preserves_unspent_exploration_budget() {
        let dir = tempfile::tempdir().unwrap();
        let telemetry = observer(dir.path().join("status.json"));
        let frontier = Frontier::open(dir.path().join("frontier.json"), 2, 50, telemetry).unwrap();
        let seed = Url::parse("https://example.org/").unwrap();
        frontier.discover([seed.clone()]).unwrap();
        let job = frontier.claim().unwrap().unwrap();
        let mut outcome = JobOutcome::default();
        outcome.record(
            seed.clone(),
            FetchOutcome::Deferred {
                reason: "robots_unavailable".into(),
                retry_after_ms: None,
            },
        );
        let now = Utc::now();
        frontier
            .resolve(&job.domain, &outcome, 10_000, now)
            .unwrap();
        let retried = frontier
            .claim_at(now + chrono::Duration::seconds(10))
            .unwrap()
            .unwrap();
        assert_eq!(retried.urls[0].url, seed);
        assert_eq!(retried.wandering_urls, 49);
    }

    #[test]
    fn extreme_retry_after_saturates_without_early_retry_or_service_failure() {
        let dir = tempfile::tempdir().unwrap();
        let telemetry = observer(dir.path().join("status.json"));
        let path = dir.path().join("frontier.json");
        let frontier = Frontier::open(path.clone(), 2, 5, telemetry.clone()).unwrap();
        let seed = Url::parse("https://example.org/").unwrap();
        frontier.discover([seed.clone()]).unwrap();
        let job = frontier.claim().unwrap().unwrap();
        let mut outcome = JobOutcome::default();
        outcome.record(
            seed,
            FetchOutcome::Deferred {
                reason: "http_429".into(),
                retry_after_ms: Some(u64::MAX),
            },
        );
        frontier
            .resolve(&job.domain, &outcome, 10_000, Utc::now())
            .unwrap();
        let persisted: Queue = serde_json::from_reader(fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(
            persisted.delayed[0].not_before,
            chrono::DateTime::<Utc>::MAX_UTC
        );
        assert!(persisted.active.is_empty());
        drop(frontier);
        let restored = Frontier::open(path, 2, 5, telemetry).unwrap();
        assert!(!restored.is_empty());
        assert!(restored
            .claim_at(chrono::DateTime::<Utc>::MAX_UTC - chrono::Duration::seconds(1))
            .unwrap()
            .is_none());
    }

    #[test]
    fn cancellation_preserves_pending_urls_and_exploration_with_courtesy_delay() {
        let dir = tempfile::tempdir().unwrap();
        let telemetry = observer(dir.path().join("status.json"));
        let path = dir.path().join("frontier.json");
        let frontier = Frontier::open(path.clone(), 2, 5, telemetry.clone()).unwrap();
        let seed = Url::parse("https://example.org/").unwrap();
        frontier.discover([seed.clone()]).unwrap();
        let job = frontier.claim().unwrap().unwrap();
        let mut outcome = JobOutcome::default();
        outcome.record(seed.clone(), FetchOutcome::Cancelled);
        let now = Utc::now();
        frontier
            .resolve(&job.domain, &outcome, 10_000, now)
            .unwrap();
        drop(frontier);
        let restored = Frontier::open(path, 2, 5, telemetry).unwrap();
        assert!(restored
            .claim_at(now + chrono::Duration::seconds(9))
            .unwrap()
            .is_none());
        let retried = restored
            .claim_at(now + chrono::Duration::seconds(10))
            .unwrap()
            .unwrap();
        assert_eq!(retried.urls[0].url, seed);
        assert_eq!(retried.wandering_urls, 4);
    }

    #[test]
    fn storage_failure_keeps_active_job_and_outcome_durable_for_replay() {
        let dir = tempfile::tempdir().unwrap();
        let telemetry = observer(dir.path().join("status.json"));
        let path = dir.path().join("frontier.json");
        let frontier = Frontier::open(path.clone(), 2, 5, telemetry.clone()).unwrap();
        let seed = Url::parse("https://example.org/").unwrap();
        frontier.discover([seed.clone()]).unwrap();
        let job = frontier.claim().unwrap().unwrap();
        let mut outcome = JobOutcome::default();
        outcome.record(
            seed.clone(),
            FetchOutcome::StorageFailure {
                message: "disk full".into(),
            },
        );
        let ledger = dir.path().join("jobs");
        fs::create_dir(&ledger).unwrap();
        persist_job_outcome(&ledger, &job.domain, &outcome).unwrap();
        assert!(frontier
            .resolve(&job.domain, &outcome, 10_000, Utc::now())
            .is_err());
        let persisted: Queue = serde_json::from_reader(fs::File::open(&path).unwrap()).unwrap();
        assert_eq!(persisted.active.len(), 1);
        assert!(persisted.delayed.is_empty());
        let record = fs::read_dir(&ledger)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let record: serde_json::Value = serde_json::from_slice(&fs::read(record).unwrap()).unwrap();
        assert_eq!(record["successful"], false);
        assert_eq!(record["outcome"]["status"], "storage_failed");
        drop(frontier);
        let restored = Frontier::open(path, 2, 5, telemetry).unwrap();
        assert_eq!(restored.claim().unwrap().unwrap().urls[0].url, seed);
    }

    #[test]
    fn legacy_frontier_without_delayed_jobs_remains_readable() {
        let dir = tempfile::tempdir().unwrap();
        let telemetry = observer(dir.path().join("status.json"));
        let path = dir.path().join("frontier.json");
        fs::write(&path, r#"{"pending":[],"active":{},"seen":[]}"#).unwrap();
        let frontier = Frontier::open(path, 2, 5, telemetry).unwrap();
        assert!(frontier.is_empty());
        assert!(frontier.claim().unwrap().is_none());
    }

    #[test]
    fn local_config_migrates_to_public_only_and_rejects_private_mode() {
        let source = include_str!("../../../../configs/crawler/local.toml");
        let mut config: Config = toml::from_str(source).unwrap();
        config.validate().unwrap();
        assert_eq!(
            config.crawler_config().network_policy,
            CrawlNetworkPolicy::PublicOnly
        );
        assert_eq!(config.crawler_config().max_redirect_hops, 10);
        let legacy = source
            .lines()
            .filter(|line| {
                !line.starts_with("network_policy") && !line.starts_with("max_redirect_hops")
            })
            .collect::<Vec<_>>()
            .join("\n");
        let migrated: Config = toml::from_str(&legacy).unwrap();
        assert_eq!(migrated.network_policy, CrawlNetworkPolicy::PublicOnly);
        assert_eq!(migrated.max_redirect_hops, 10);
        config.network_policy = CrawlNetworkPolicy::AllowPrivate;
        assert!(config.validate().is_err());
    }

    #[test]
    fn metrics_destination_is_independent_of_durable_state_and_legacy_paths_work() {
        let source = include_str!("../../../../configs/crawler/local.toml");
        let mut config: Config = toml::from_str(source).unwrap();
        let original_state = config.state_directory().to_owned();
        let original_lock = config.lock_path();
        config.status_path = PathBuf::from("/unavailable-metrics/status.json");
        assert_eq!(config.state_directory(), original_state);
        assert_eq!(config.lock_path(), original_lock);

        let legacy = source
            .lines()
            .filter(|line| !line.starts_with("state_directory"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut config: Config = toml::from_str(&legacy).unwrap();
        assert_eq!(config.state_directory(), original_state);
        assert_eq!(config.lock_path(), original_lock);
        config.status_path = PathBuf::from("legacy/custom.json");
        assert_eq!(config.state_directory(), Path::new("legacy"));
        assert_eq!(config.lock_path(), PathBuf::from("legacy/custom.lock"));
    }

    fn test_datum() -> CrawlDatum {
        CrawlDatum {
            url: Url::parse("https://example.org/article").unwrap(),
            payload_type: warc::PayloadType::Html,
            body: "<html><body>durable page despite metrics failure</body></html>".into(),
            fetch_time_ms: 42,
            date: Utc::now(),
        }
    }

    #[tokio::test]
    async fn failed_metrics_output_does_not_fail_warc_checkpoint_or_job_commit() {
        let dir = tempfile::tempdir().unwrap();
        let status = dir.path().join("status.json");
        fs::create_dir(&status).unwrap();
        let telemetry = observer(status);
        assert!(telemetry.flush().is_err());
        let frontier = Arc::new(
            Frontier::open(dir.path().join("frontier.json"), 2, 5, telemetry.clone()).unwrap(),
        );
        let datum = test_datum();
        frontier.discover([datum.url.clone()]).unwrap();
        let job = frontier.claim().unwrap().unwrap();
        telemetry.register_site(job.domain.as_str().into(), 5);
        let warcs = dir.path().join("warc");
        fs::create_dir(&warcs).unwrap();
        let sink = LocalWarcSink {
            directory: warcs.clone(),
            observer: telemetry.clone(),
            frontier: frontier.clone(),
            fatal_error: Mutex::new(None),
            store_warc: atomic_write,
        };
        sink.write(datum.clone()).await.unwrap();
        sink.checkpoint().await.unwrap();
        sink.finish().await.unwrap();
        let mut outcome = JobOutcome::default();
        outcome.record(
            datum.url.clone(),
            FetchOutcome::DurableSuccess {
                final_url: datum.url.clone(),
            },
        );
        let ledger = dir.path().join("jobs");
        fs::create_dir(&ledger).unwrap();
        persist_job_outcome(&ledger, &job.domain, &outcome).unwrap();
        frontier
            .resolve(&job.domain, &outcome, 10_000, Utc::now())
            .unwrap();
        assert!(frontier.is_empty());
        let persisted: Queue = serde_json::from_slice(&fs::read(&frontier.path).unwrap()).unwrap();
        assert!(persisted.active.is_empty());
        let path = fs::read_dir(warcs).unwrap().next().unwrap().unwrap().path();
        let file = warc::WarcFile::open(&path).unwrap();
        let record = file.records().next().unwrap().unwrap();
        assert_eq!(record.request.url, datum.url.as_str());
        assert_eq!(record.response.body, datum.body);
        let path = fs::read_dir(ledger)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let record: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(record["successful"], true);
        assert!(telemetry.check().is_err());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn kernel_disk_full_stops_warc_writes_and_retains_job_for_replay() {
        fn full_device(_path: &Path, bytes: &[u8]) -> Result<()> {
            fs::OpenOptions::new()
                .write(true)
                .open("/dev/full")?
                .write_all(bytes)?;
            Ok(())
        }
        let dir = tempfile::tempdir().unwrap();
        let telemetry = observer(dir.path().join("status.json"));
        let path = dir.path().join("frontier.json");
        let frontier = Arc::new(Frontier::open(path.clone(), 2, 5, telemetry.clone()).unwrap());
        let datum = test_datum();
        frontier.discover([datum.url.clone()]).unwrap();
        let job = frontier.claim().unwrap().unwrap();
        telemetry.register_site(job.domain.as_str().into(), 5);
        let warcs = dir.path().join("warc");
        fs::create_dir(&warcs).unwrap();
        let mut sink = LocalWarcSink {
            directory: warcs.clone(),
            observer: telemetry.clone(),
            frontier: frontier.clone(),
            fatal_error: Mutex::new(None),
            store_warc: full_device,
        };
        let error = sink.write(datum.clone()).await.unwrap_err();
        match &error {
            super::super::Error::Anyhow(error) => {
                assert_eq!(
                    error
                        .downcast_ref::<std::io::Error>()
                        .unwrap()
                        .raw_os_error(),
                    Some(28)
                );
            }
            other => panic!("Expected kernel ENOSPC, got {other:?}"),
        }
        assert!(sink.checkpoint().await.is_err());
        // Even if storage recovers, this failed sink must not accept more work.
        sink.store_warc = atomic_write;
        assert!(sink.write(datum.clone()).await.is_err());
        assert!(sink.finish().await.is_err());
        assert_eq!(fs::read_dir(warcs).unwrap().count(), 0);
        let mut outcome = JobOutcome::default();
        outcome.record(
            datum.url.clone(),
            FetchOutcome::StorageFailure {
                message: error.to_string(),
            },
        );
        let ledger = dir.path().join("jobs");
        fs::create_dir(&ledger).unwrap();
        persist_job_outcome(&ledger, &job.domain, &outcome).unwrap();
        assert!(frontier
            .resolve(&job.domain, &outcome, 10_000, Utc::now())
            .is_err());
        let persisted: Queue = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted.active.len(), 1);
        assert!(persisted.delayed.is_empty());
        telemetry.flush().unwrap();
        let snapshot: serde_json::Value =
            serde_json::from_slice(&fs::read(dir.path().join("status.json")).unwrap()).unwrap();
        assert_eq!(snapshot["counters"]["downloaded"], 0);
        assert_eq!(snapshot["warc"]["files"], 0);
        drop(sink);
        drop(frontier);
        let restored = Frontier::open(path, 2, 5, telemetry).unwrap();
        assert_eq!(restored.claim().unwrap().unwrap().urls[0].url, datum.url);
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
