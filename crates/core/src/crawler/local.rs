//! Standalone production crawler: the batch executor, a durable web frontier,
//! durable WARC segments, and observations consumed by the Svelte control panel.

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
};

use super::{
    extraction::{CrawlExtraction, ExtractionContext, Extractor, VisitedUrls},
    observer::{Counters, FrontierState, Observer, Snapshot, Warc},
    outcome::{FetchOutcome, JobOutcome, JobStatus},
    robot_client::{validate_seed_urls, RobotClient},
    CrawlDatum, DatumSink, Domain, Job, JobExecutor, WeightedUrl,
};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub retry: super::retry::Config,
    #[serde(default)]
    pub recrawl: super::recrawl::Config,
    #[serde(default)]
    pub discovery: super::discovery::Config,
    #[serde(default)]
    pub traps: super::traps::Config,
    #[serde(default)]
    pub seeds: Vec<String>,
    pub seeds_file: Option<PathBuf>,
    #[serde(default)]
    pub plan_file: Option<PathBuf>,
    pub output_path: PathBuf,
    pub status_path: PathBuf,
    pub state_directory: Option<PathBuf>,
    pub pages_per_domain: u64,
    pub num_worker_threads: usize,
    pub min_crawl_delay_ms: u64,
    pub timeout_seconds: u64,
    pub user_agent: UserAgent,
    #[serde(default = "super::scheduler::Config::local")]
    pub scheduler: super::scheduler::Config,
    #[serde(default)]
    pub robots: super::robots_txt::Config,
    #[serde(default)]
    pub network_policy: CrawlNetworkPolicy,
    #[serde(default)]
    pub transport: super::transport::TransportConfig,
    #[serde(default)]
    pub body: super::body::BodyConfig,
    #[serde(default)]
    pub pipeline: super::pipeline::PipelineConfig,
    #[serde(default)]
    pub spool: Option<super::spool::SpoolConfig>,
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
    fn spool_config(&self) -> super::spool::SpoolConfig {
        self.spool
            .clone()
            .unwrap_or_else(|| super::spool::SpoolConfig {
                directory: self.output_path.join("spool"),
                ..Default::default()
            })
    }
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
        self.retry.validate()?;
        self.recrawl.validate()?;
        self.discovery.validate()?;
        self.traps.validate()?;
        self.scheduler.validate()?;
        self.transport.validate()?;
        self.body.validate()?;
        self.pipeline.validate_body(&self.body)?;
        self.spool_config().validate()?;
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
            discovery: self.discovery.clone(),
            retry: self.retry.clone(),
            recrawl: self.recrawl.clone(),
            scheduler: self.scheduler.clone(),
            robots: self.robots.clone(),
            user_agent: self.user_agent.clone(),
            network_policy: self.network_policy,
            transport: self.transport.clone(),
            body: self.body.clone(),
            pipeline: self.pipeline.clone(),
            spool: self.spool_config(),
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
pub(super) struct DelayedJob {
    pub(super) job: Job,
    pub(super) not_before: chrono::DateTime<Utc>,
}

use super::frontier::Frontier;

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

#[derive(Clone)]
pub(super) struct LocalWarcSink {
    directory: PathBuf,
    observer: Arc<Observer>,
    frontier: Arc<Frontier>,
    fatal_error: Arc<Mutex<Option<String>>>,
    extractor: Extractor,
    pipeline: super::pipeline::Pipeline,
    store_warc: fn(&Path, &[u8]) -> Result<()>,
    segmented: Option<Arc<super::WarcWriter>>,
}

impl LocalWarcSink {
    fn commit(&self, datum: &CrawlDatum, extraction: Option<&CrawlExtraction>) -> Result<()> {
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
                capture: None,
                fetch_time_ms: datum.fetch_time_ms,
            },
        })?;
        let bytes = writer.finish()?;
        let filename = format!("{}.warc.gz", uuid::Uuid::new_v4());
        (self.store_warc)(&self.directory.join(&filename), &bytes)?;
        if let Some(extraction) = extraction {
            self.frontier
                .discover(extraction.discovery_urls.iter().cloned())?;
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
        if let Some(writer) = &self.segmented {
            let stats = writer.metrics.stats();
            anyhow::ensure!(
                stats.errors == 0,
                "Crawler storage failed: {:?}",
                stats.last_error
            );
        }
        if let Some(error) = self.fatal_error.lock().unwrap().as_ref() {
            anyhow::bail!("Crawler storage failed: {error}");
        }
        Ok(())
    }
}

impl DatumSink for LocalWarcSink {
    async fn revalidation_state(
        &self,
        url: &Url,
    ) -> super::Result<Option<super::recrawl::Resource>> {
        let frontier = self.frontier.clone();
        let url = url.clone();
        Ok(self
            .pipeline
            .control
            .run(url.as_str().len(), move || {
                frontier.revalidation_state(&url)
            })
            .await??)
    }

    async fn ingest_sitemap(
        &self,
        source: Url,
        body: super::body::AdmittedBody,
    ) -> super::Result<Option<super::discovery::Report>> {
        let frontier = self.frontier.clone();
        let bytes = body.len();
        let result = self
            .pipeline
            .parse
            .run(bytes, move || frontier.ingest_sitemap(&source, &body))
            .await??;
        Ok(Some(result))
    }

    async fn authorize_request(
        &self,
        requested: &Url,
        destination: &Url,
        now: chrono::DateTime<Utc>,
    ) -> super::Result<()> {
        if self.segmented.is_none() {
            return Ok(());
        }
        let frontier = self.frontier.clone();
        let requested = requested.clone();
        let destination = destination.clone();
        self.pipeline
            .control
            .run_durable(
                requested.as_str().len() + destination.as_str().len(),
                move || frontier.authorize_request(&requested, &destination, now),
            )
            .await??
    }

    async fn redirect_continuation(
        &self,
        url: &Url,
    ) -> super::Result<Option<super::RedirectContinuation>> {
        let frontier = self.frontier.clone();
        let url = url.clone();
        Ok(self
            .pipeline
            .control
            .run(url.as_str().len(), move || {
                frontier.redirect_continuation(&url)
            })
            .await??)
    }
    async fn save_redirect_continuation(
        &self,
        url: &Url,
        continuation: super::RedirectContinuation,
    ) -> super::Result<()> {
        let frontier = self.frontier.clone();
        let url = url.clone();
        let bytes = serde_json::to_vec(&continuation)
            .map_err(anyhow::Error::from)?
            .len();
        self.pipeline
            .control
            .run_durable(bytes, move || {
                frontier.save_redirect_continuation(&url, continuation)
            })
            .await??;
        Ok(())
    }
    async fn is_sitemap(&self, url: &Url) -> super::Result<bool> {
        let frontier = self.frontier.clone();
        let url = url.clone();
        Ok(self
            .pipeline
            .control
            .run(url.as_str().len(), move || frontier.is_sitemap(&url))
            .await??)
    }
    async fn discovered_urls(&self, urls: Vec<Url>, sitemaps: Vec<Url>) -> super::Result<()> {
        if urls.is_empty() && sitemaps.is_empty() {
            return Ok(());
        }
        let frontier = self.frontier.clone();
        let bytes = urls
            .iter()
            .chain(&sitemaps)
            .map(|url| url.as_str().len())
            .sum();
        self.pipeline
            .control
            .run_durable(bytes, move || {
                frontier.admit_urls(urls)?;
                frontier.admit_sitemaps(sitemaps)
            })
            .await??;
        Ok(())
    }
    async fn reserve_url(&self, url: &Url) -> super::Result<Option<String>> {
        if self.segmented.is_none() {
            return Ok(Some(uuid::Uuid::new_v4().to_string()));
        }
        let frontier = self.frontier.clone();
        let url = url.clone();
        Ok(self
            .pipeline
            .control
            .run_durable(url.as_str().len(), move || frontier.begin_url(&url))
            .await??)
    }
    async fn write(&self, datum: CrawlDatum) -> super::Result<()> {
        self.check().map_err(super::Error::Anyhow)?;
        let admission = self.pipeline.admit(datum.body.len()).await?;
        let extractor = self.extractor.clone();
        let page = self
            .pipeline
            .parse
            .run(datum.body.len(), move || {
                let extraction = extractor
                    .parse(
                        &datum.body,
                        &datum.url,
                        ExtractionContext {
                            domain: &Domain::from(&datum.url),
                            crawled_urls: &VisitedUrls::default(),
                            requested_url: &datum.url,
                        },
                    )
                    .ok();
                super::pipeline::PreparedPage {
                    datum,
                    extraction,
                    body_lease: None,
                    admission: Some(admission),
                }
            })
            .await?;
        self.write_prepared(page).await
    }
    async fn write_extracted(
        &self,
        datum: CrawlDatum,
        extraction: Option<&CrawlExtraction>,
    ) -> super::Result<()> {
        let admission = self.pipeline.admit(datum.body.len()).await?;
        let mut page = super::pipeline::PreparedPage::unleased(datum, extraction.cloned());
        page.admission = Some(admission);
        self.write_prepared(page).await
    }
    async fn write_prepared(&self, page: super::pipeline::PreparedPage) -> super::Result<()> {
        if let Some(writer) = &self.segmented {
            return writer.write_prepared(page).await;
        }
        self.check().map_err(super::Error::Anyhow)?;
        let sink = self.clone();
        self.pipeline
            .storage
            .run_durable(page.datum.body.len(), move || {
                sink.check()?;
                let result = sink.commit(&page.datum, page.extraction.as_ref());
                if let Err(error) = &result {
                    *sink.fatal_error.lock().unwrap() = Some(error.to_string());
                    sink.pipeline.stop_admission();
                }
                drop(page);
                result
            })
            .await?
            .map_err(super::Error::Anyhow)
    }
    async fn finish(&self) -> super::Result<()> {
        if let Some(writer) = &self.segmented {
            return writer.finish().await;
        }
        self.check().map_err(super::Error::Anyhow)
    }
    async fn checkpoint(&self) -> super::Result<()> {
        if let Some(writer) = &self.segmented {
            return writer.checkpoint().await;
        }
        self.check().map_err(super::Error::Anyhow)
    }
}

#[cfg(feature = "crawler-bench")]
pub(super) fn benchmark_sink(
    directory: PathBuf,
    observer: Arc<Observer>,
    extractor: Extractor,
    pipeline: super::pipeline::Pipeline,
) -> Result<Arc<LocalWarcSink>> {
    fs::create_dir_all(&directory)?;
    let frontier = Arc::new(Frontier::open(
        directory.with_extension("frontier.json"),
        256,
        1,
        observer.clone(),
    )?);
    Ok(Arc::new(LocalWarcSink {
        directory,
        observer,
        frontier,
        fatal_error: Arc::new(Mutex::new(None)),
        extractor,
        pipeline,
        store_warc: atomic_write,
        segmented: None,
    }))
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
    let spool_config = config.spool_config();
    let directory = spool_config.directory.join("segments");
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
    observer.observe_extraction(client.extractor().clone());
    let frontier = Arc::new(Frontier::open(
        config.state_directory().join("frontier.json"),
        config.max_frontier,
        config.pages_per_domain,
        observer.clone(),
    )?);
    frontier.configure_retry(&config.retry)?;
    frontier.configure_recrawl(&config.recrawl)?;
    frontier.configure_discovery(&config.discovery, &config.traps)?;
    observer.observe_frontier(frontier.metrics.clone());
    observer.observe_robots(client.robots_txt_manager().clone());
    frontier.discover(seeds.clone())?;
    if let Some(path) = &config.plan_file {
        frontier.import_plan(path)?;
        frontier.promote_candidates()?;
    }
    let commit_frontier = frontier.clone();
    let commit_observer = observer.clone();
    let segmented = Arc::new(
        super::WarcWriter::open(
            spool_config,
            None,
            client.pipeline().clone(),
            Some(Arc::new(move |page, receipt| {
                if let Some(capture) = &page.datum.capture {
                    commit_frontier.commit_capture(&capture.metadata, receipt, false)?;
                    commit_frontier.drain_outbox()?;
                } else if let Some(extraction) = &page.extraction {
                    commit_frontier.discover(extraction.discovery_urls.iter().cloned())?;
                }
                commit_observer.saved(
                    &Domain::from(&page.datum.url),
                    &page.datum,
                    receipt.segment.clone(),
                    receipt.end - receipt.start,
                );
                Ok(())
            })),
        )
        .await?,
    );
    observer.observe_spool(segmented.metrics.clone());
    let recovery_frontier = frontier.clone();
    let recovery_path = config.spool_config().directory;
    client
        .pipeline()
        .control
        .run_durable(0, move || recovery_frontier.reconcile_spool(&recovery_path))
        .await??;
    let sink = Arc::new(LocalWarcSink {
        directory,
        observer: observer.clone(),
        frontier: frontier.clone(),
        fatal_error: Arc::new(Mutex::new(None)),
        extractor: client.extractor().clone(),
        pipeline: client.pipeline().clone(),
        store_warc: atomic_write,
        segmented: Some(segmented),
    });
    let job_directory = config.state_directory().join("jobs").join(&run_id);
    fs::create_dir_all(&job_directory)?;
    let pipeline = client.pipeline().clone();
    pipeline.start_monitor();
    observer.observe_pipeline(pipeline.clone());
    let mut jobs = tokio::task::JoinSet::new();
    let mut claims = tokio::task::JoinSet::new();
    let cancellation = Arc::new(AtomicBool::new(false));
    let mut shutting_down = false;
    let mut can_claim = true;
    let mut claim_due = tokio::time::Instant::now();
    let mut heartbeat = tokio::time::interval(Duration::from_secs(2));
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);
    let mut exhausted_at: Option<std::time::Instant> = None;
    let seeds = Arc::new(seeds);
    let control_directory = Arc::new(config.state_directory().join("control"));
    let seed_bytes = seeds.iter().map(|url| url.as_str().len()).sum::<usize>();
    let outcome: Result<()> = async {
        loop {
            if !shutting_down && can_claim && claims.is_empty()
                && jobs.len() < config.num_worker_threads {
                can_claim = false;
                let frontier = frontier.clone();
                let control = pipeline.control.clone();
                let seeds = seeds.clone();
                let control_directory = control_directory.clone();
                let fair_turns = config.scheduler.mode == super::scheduler::Mode::FairTurns;
                let restart = exhausted_at.is_some_and(|since| since.elapsed() >= Duration::from_secs(config.revisit_interval_seconds));
                claims.spawn(async move {
                    control.run_durable(seed_bytes, move || {
                        frontier.process_controls(&control_directory)?;
                        frontier.requeue_recrawls(Utc::now())?;
                        if restart && frontier.is_empty() { frontier.restart_round(seeds.as_ref().clone())?; }
                        frontier.promote_candidates()?;
                        let job = if fair_turns { frontier.claim_turn_at(Utc::now())? } else { frontier.claim()? };
                        Ok::<_, anyhow::Error>((job, frontier.is_empty(), frontier.next_eligible_at()?))
                    }).await.map_err(anyhow::Error::from)?
                });
            }
            if shutting_down && jobs.is_empty() && claims.is_empty() { break; }
            tokio::select! {
                stopped = &mut shutdown, if !shutting_down => {
                    stopped?; shutting_down = true;
                    cancellation.store(true, Ordering::Release);
                    pipeline.stop_admission();
                }
                _ = heartbeat.tick() => { observer.heartbeat(); if !shutting_down { can_claim = true; } }
                _ = tokio::time::sleep_until(claim_due), if !can_claim && claims.is_empty() && jobs.len() < config.num_worker_threads && !shutting_down => { can_claim = true; }
                result = claims.join_next(), if !claims.is_empty() => {
                    let (job, empty, eligible) = result.unwrap().context("Frontier task panicked")??;
                    claim_due = tokio::time::Instant::now() + Duration::from_millis(eligible.map_or(1000, |due| due.saturating_sub(Utc::now().timestamp_millis().max(0) as u64)).max(1));
                    if let Some(job) = job {
                        exhausted_at = None;
                        observer.phase("running");
                        observer.register_site(job.domain.as_str().into(), config.pages_per_domain);
                        let domain = job.domain.clone();
                        let executor = JobExecutor::new(job.into(), crawler_config.clone(), sink.clone(), client.clone())
                            .with_observer(observer.clone()).with_cancellation(cancellation.clone());
                        let control = pipeline.control.clone();
                        let directory = job_directory.clone();
                        let frontier = frontier.clone();
                        let sink = sink.clone();
                        let minimum = config.min_crawl_delay_ms;
                        let crawler_fair = config.scheduler.mode == super::scheduler::Mode::FairTurns;
                        jobs.spawn(async move {
                            let outcome = executor.run().await;
                            let bytes = outcome.urls.iter().map(|entry| entry.url.as_str().len() + 128).sum::<usize>();
                            control.run_durable(bytes, move || {
                                persist_job_outcome(&directory, &domain, &outcome)?;
                                if crawler_fair { frontier.resolve_turn(&domain, &outcome, minimum, Utc::now())?; }
                                else { frontier.resolve(&domain, &outcome, minimum, Utc::now())?; }
                                sink.check()?;
                                Ok::<_, anyhow::Error>(())
                            }).await.map_err(anyhow::Error::from)?
                        });
                        can_claim = true;
                    } else if jobs.is_empty() {
                        observer.phase("idle");
                        if empty { exhausted_at.get_or_insert_with(std::time::Instant::now); }
                        else { exhausted_at = None; }
                    }
                }
                result = jobs.join_next(), if !jobs.is_empty() => {
                    result.unwrap().context("Crawler worker panicked")??;
                    can_claim = true;
                }
            }
        }
        Ok(())
    }.await;
    pipeline.stop_admission();
    if outcome.is_err() {
        cancellation.store(true, Ordering::Release);
        jobs.abort_all();
        claims.abort_all();
        while jobs.join_next().await.is_some() {}
        while claims.join_next().await.is_some() {}
    }
    let finished = sink.finish().await;
    pipeline.shutdown().await;
    observer.finish(if outcome.is_ok() && finished.is_ok() {
        "stopped"
    } else {
        "failed"
    });
    outcome?;
    finished?;
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
        assert_eq!(frontier.metrics.snapshot().pending_sites, 1);
        assert_eq!(frontier.metrics.snapshot().active_sites, 0);
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
        assert_eq!(frontier.metrics.snapshot().active_sites, 1);
        assert_eq!(frontier.metrics.snapshot().pending_sites, 0);
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
            capture: None,
            url: Url::parse("https://example.org/article").unwrap(),
            payload_type: warc::PayloadType::Html,
            body: "<html><body>durable page despite metrics failure</body></html>".into(),
            fetch_time_ms: 42,
            date: Utc::now(),
        }
    }

    #[tokio::test]
    async fn segmented_local_sink_keeps_frontier_observer_and_reader_contracts() {
        let dir = tempfile::tempdir().unwrap();
        let status = dir.path().join("status.json");
        let telemetry = observer(status.clone());
        let frontier = Arc::new(
            Frontier::open(dir.path().join("frontier.json"), 8, 5, telemetry.clone()).unwrap(),
        );
        let pipeline = super::super::pipeline::Pipeline::new(Default::default()).unwrap();
        let extractor = Extractor::default();
        let archive = dir.path().join("spool");
        let commit_frontier = frontier.clone();
        let commit_observer = telemetry.clone();
        let writer = Arc::new(
            super::super::WarcWriter::open(
                super::super::spool::SpoolConfig {
                    directory: archive.clone(),
                    ..Default::default()
                },
                None,
                pipeline.clone(),
                Some(Arc::new(move |page, receipt| {
                    if let Some(extraction) = &page.extraction {
                        commit_frontier.discover(extraction.discovery_urls.iter().cloned())?;
                    }
                    commit_observer.saved(
                        &Domain::from(&page.datum.url),
                        &page.datum,
                        receipt.segment.clone(),
                        receipt.end - receipt.start,
                    );
                    Ok(())
                })),
            )
            .await
            .unwrap(),
        );
        telemetry.observe_spool(writer.metrics.clone());
        let sink = LocalWarcSink {
            directory: archive.join("segments"),
            observer: telemetry.clone(),
            frontier: frontier.clone(),
            fatal_error: Arc::new(Mutex::new(None)),
            extractor: extractor.clone(),
            pipeline: pipeline.clone(),
            store_warc: atomic_write,
            segmented: Some(writer),
        };
        for index in 0..2 {
            let mut datum = test_datum();
            datum.url = Url::parse(&format!("https://example.org/article/{index}")).unwrap();
            datum.body = "<base href='/base/'><meta name='robots' content='noindex'><a href='https://external.org/page'>other</a><a href='next'>internal</a>".into();
            let extraction = extractor
                .parse(
                    &datum.body,
                    &datum.url,
                    ExtractionContext {
                        domain: &Domain::from(&datum.url),
                        crawled_urls: &VisitedUrls::default(),
                        requested_url: &datum.url,
                    },
                )
                .unwrap();
            sink.write_prepared(super::super::pipeline::PreparedPage::unleased(
                datum,
                Some(extraction),
            ))
            .await
            .unwrap();
        }
        sink.checkpoint().await.unwrap();
        sink.finish().await.unwrap();
        telemetry.flush().unwrap();
        let snapshot: serde_json::Value =
            serde_json::from_slice(&fs::read(status).unwrap()).unwrap();
        assert_eq!(snapshot["counters"]["downloaded"], 2);
        assert_eq!(snapshot["warc"]["files"], 1);
        assert_eq!(snapshot["spool"]["records_durable"], 2);
        assert_eq!(extractor.stats().documents, 2);
        let discovered: Vec<_> = [
            frontier.claim().unwrap().unwrap(),
            frontier.claim().unwrap().unwrap(),
        ]
        .into_iter()
        .flat_map(|job| job.urls.into_iter().map(|url| url.url.to_string()))
        .collect();
        assert!(discovered.contains(&"https://external.org/page".into()));
        assert!(discovered.contains(&"https://example.org/base/next".into()));
        let segment = fs::read_dir(archive.join("segments"))
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "gz"))
            .unwrap();
        assert_eq!(
            warc::WarcFile::open(segment)
                .unwrap()
                .records()
                .filter(|r| r.is_ok())
                .count(),
            2
        );
        pipeline.shutdown().await;
    }

    #[tokio::test]
    async fn shared_extraction_discovers_domains_without_a_sink_parse_or_changed_warc() {
        let dir = tempfile::tempdir().unwrap();
        let telemetry = observer(dir.path().join("status.json"));
        let frontier = Arc::new(
            Frontier::open(dir.path().join("frontier.json"), 8, 5, telemetry.clone()).unwrap(),
        );
        let extractor = Extractor::default();
        let mut datum = test_datum();
        datum.body = "<base href='/base/'><meta name='robots' content='noindex'><a href='https://external.org/page'>other</a><a href='next'>internal</a>".into();
        let extraction = extractor
            .parse(
                &datum.body,
                &datum.url,
                ExtractionContext {
                    domain: &Domain::from(&datum.url),
                    crawled_urls: &VisitedUrls::default(),
                    requested_url: &datum.url,
                },
            )
            .unwrap();
        assert!(extraction.no_index);
        let sink = LocalWarcSink {
            directory: dir.path().to_path_buf(),
            observer: telemetry,
            frontier: frontier.clone(),
            fatal_error: Arc::new(Mutex::new(None)),
            extractor: extractor.clone(),
            pipeline: super::super::pipeline::Pipeline::new(Default::default()).unwrap(),
            store_warc: atomic_write,
            segmented: None,
        };
        sink.write_extracted(datum.clone(), Some(&extraction))
            .await
            .unwrap();
        sink.checkpoint().await.unwrap();
        assert_eq!(
            extractor.stats().documents,
            1,
            "storage must reuse the worker's extraction"
        );
        let first = frontier.claim().unwrap().unwrap();
        assert_eq!(first.urls[0].url.as_str(), "https://external.org/page");
        let second = frontier.claim().unwrap().unwrap();
        assert_eq!(second.urls[0].url.as_str(), "https://example.org/base/next");
        let warc_path = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "gz"))
            .unwrap();
        let file = warc::WarcFile::open(&warc_path).unwrap();
        let record = file.records().next().unwrap().unwrap();
        assert_eq!(record.response.body, datum.body);
        assert_eq!(record.request.url, datum.url.as_str());
    }

    #[tokio::test]
    async fn accepted_local_write_retains_admission_and_survives_waiter_cancellation() {
        let dir = tempfile::tempdir().unwrap();
        let telemetry = observer(dir.path().join("status.json"));
        let pipeline = super::super::pipeline::Pipeline::new(Default::default()).unwrap();
        let frontier = Arc::new(
            Frontier::open(dir.path().join("frontier.json"), 8, 5, telemetry.clone()).unwrap(),
        );
        let warcs = dir.path().join("warc");
        fs::create_dir(&warcs).unwrap();
        let sink = Arc::new(LocalWarcSink {
            directory: warcs.clone(),
            observer: telemetry,
            frontier,
            fatal_error: Arc::new(Mutex::new(None)),
            extractor: Extractor::default(),
            pipeline: pipeline.clone(),
            store_warc: atomic_write,
            segmented: None,
        });
        let (release, blocked) = std::sync::mpsc::channel();
        let holding = {
            let stage = pipeline.storage.clone();
            tokio::spawn(async move { stage.run_durable(1, move || blocked.recv().unwrap()).await })
        };
        while pipeline.stats().storage.running == 0 {
            tokio::task::yield_now().await;
        }
        let datum = test_datum();
        let mut page = super::super::pipeline::PreparedPage::unleased(datum.clone(), None);
        page.admission = Some(pipeline.admit(datum.body.len()).await.unwrap());
        let pending = tokio::spawn(async move { sink.write_prepared(page).await });
        while pipeline.stats().storage.items < 2 {
            tokio::task::yield_now().await;
        }
        pending.abort();
        let _ = pending.await;
        assert_eq!(pipeline.stats().admission.items, 1);
        release.send(()).unwrap();
        holding.await.unwrap().unwrap();
        pipeline.shutdown().await;
        assert_eq!(pipeline.stats().admission.items, 0);
        let path = fs::read_dir(warcs).unwrap().next().unwrap().unwrap().path();
        let file = warc::WarcFile::open(path).unwrap();
        assert_eq!(
            file.records().next().unwrap().unwrap().response.body,
            datum.body
        );
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
            fatal_error: Arc::new(Mutex::new(None)),
            extractor: Extractor::default(),
            pipeline: super::super::pipeline::Pipeline::new(Default::default()).unwrap(),
            store_warc: atomic_write,
            segmented: None,
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
        assert_eq!(frontier.metrics.snapshot().active_sites, 0);
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
            fatal_error: Arc::new(Mutex::new(None)),
            extractor: Extractor::default(),
            pipeline: super::super::pipeline::Pipeline::new(Default::default()).unwrap(),
            store_warc: full_device,
            segmented: None,
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
        assert_eq!(frontier.metrics.snapshot().active_sites, 1);
        assert_eq!(frontier.metrics.snapshot().pending_sites, 0);
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
                metadata: warc::Metadata {
                    capture: None,
                    fetch_time_ms: 42,
                },
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
