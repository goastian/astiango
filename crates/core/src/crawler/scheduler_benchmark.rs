use super::{
    frontier::Frontier,
    pipeline::Pipeline,
    robot_client::RobotClient,
    runtime::{IoFuture, SystemClock},
    scheduler::Mode,
    spool::{Spool, SpoolConfig, SpoolMetrics},
    transport::{DnsAnswer, FixturePolicy, SharedTransport, TransportResolver},
    CrawlDatum, DatumSink, JobExecutor,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use url::Url;

#[derive(Deserialize, Serialize)]
pub struct Config {
    pub directory: PathBuf,
    pub port: u16,
    pub origins: usize,
    pub pages: usize,
    pub workers: usize,
    pub delay_ms: u64,
    pub mode: Mode,
}
struct Resolver;
impl TransportResolver for Resolver {
    fn resolve<'a>(&'a self, _: &'a str) -> IoFuture<'a, super::Result<DnsAnswer>> {
        Box::pin(async {
            Ok(DnsAnswer {
                addresses: vec!["127.0.0.1".parse().unwrap()],
                ttl: Duration::from_secs(60),
            })
        })
    }
}
struct Sink {
    frontier: Arc<Frontier>,
    spool: Arc<Mutex<Spool>>,
    pipeline: Pipeline,
}
impl DatumSink for Sink {
    async fn reserve_url(&self, url: &Url) -> super::Result<Option<String>> {
        let frontier = self.frontier.clone();
        let url = url.clone();
        Ok(self
            .pipeline
            .control
            .run_durable(url.as_str().len(), move || frontier.begin_url(&url))
            .await??)
    }
    async fn write(&self, datum: CrawlDatum) -> super::Result<()> {
        let spool = self.spool.clone();
        let frontier = self.frontier.clone();
        self.pipeline
            .storage
            .run_durable(datum.memory_bytes(), move || {
                let receipt = spool.lock().unwrap().append(&[&datum])?.remove(0);
                frontier.commit_capture(
                    &datum
                        .capture
                        .context("benchmark requires real capture")?
                        .metadata,
                    &receipt,
                    false,
                )?;
                frontier.drain_outbox()
            })
            .await??;
        Ok(())
    }
    async fn finish(&self) -> super::Result<()> {
        Ok(())
    }
}

pub async fn run(config: Config) -> Result<serde_json::Value> {
    anyhow::ensure!(
        [2, 32, 128].contains(&config.origins)
            && config.workers == 2
            && (1..=8).contains(&config.pages)
            && (20..=1000).contains(&config.delay_ms)
            && config.port > 0,
        "invalid controlled scheduler profile"
    );
    anyhow::ensure!(
        !config.directory.exists(),
        "benchmark requires a fresh directory"
    );
    fs::create_dir_all(&config.directory)?;
    let mut options = crate::config::CrawlerConfig::for_tests();
    options.network_policy = crate::config::CrawlNetworkPolicy::AllowPrivate;
    options.user_agent.token = "SchedulerBench".into();
    options.scheduler.mode = config.mode;
    options.scheduler.max_active_per_ip = 8;
    options.min_crawl_delay_ms = config.delay_ms;
    options.start_politeness_factor = 0;
    options.min_politeness_factor = 0;
    options.max_crawl_delay_ms = config.delay_ms;
    options.num_worker_threads = config.workers;
    let transport = SharedTransport::with_fixture_resolver(
        &options,
        Default::default(),
        Arc::new(Resolver),
        Arc::new(SystemClock),
        FixturePolicy {
            allowed_hosts: (0..config.origins)
                .map(|i| format!("site-{i}.test"))
                .collect::<BTreeSet<_>>(),
            certificate_pem: None,
        },
    )?;
    let client = RobotClient::with_transport(&options, transport.clone());
    let pipeline = client.pipeline().clone();
    pipeline.start_monitor();
    let frontier = Arc::new(Frontier::open_database(
        config.directory.join("frontier.json"),
        256,
        config.pages as u64,
    )?);
    let mut seeds = VecDeque::new();
    for i in 0..config.origins {
        for page in 0..config.pages {
            seeds.push_back(Url::parse(&format!(
                "http://site-{i}.test:{}/page/{page}",
                config.port
            ))?);
        }
    }
    frontier.admit_urls(seeds)?;
    let metrics = SpoolMetrics::default();
    let spool = Arc::new(Mutex::new(Spool::open(
        SpoolConfig {
            directory: config.directory.join("warc"),
            ..Default::default()
        },
        None,
        metrics.clone(),
    )?));
    let sink = Arc::new(Sink {
        frontier: frontier.clone(),
        spool,
        pipeline: pipeline.clone(),
    });
    let options = Arc::new(options);
    let cpu_started = cpu_ms()?;
    let started = Instant::now();
    let started_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let mut tasks = tokio::task::JoinSet::new();
    let mut ready_lags = Vec::new();
    let mut turns = 0u64;
    while !frontier.is_empty() || !tasks.is_empty() {
        while tasks.len() < config.workers {
            let state = frontier.clone();
            let mode = config.mode;
            let (job, due) = pipeline
                .control
                .run_durable(0, move || {
                    let due = state.next_eligible_at()?;
                    let job = if mode == Mode::FairTurns {
                        state.claim_turn_at(chrono::Utc::now())?
                    } else {
                        state.claim()?
                    };
                    Ok::<_, anyhow::Error>((job, due))
                })
                .await??;
            let Some(job) = job else {
                break;
            };
            ready_lags.push(
                chrono::Utc::now().timestamp_millis().max(0) as u64
                    - due.unwrap_or(started_ms).max(started_ms),
            );
            turns += 1;
            let domain = job.domain.clone();
            let state = frontier.clone();
            let control = pipeline.control.clone();
            let executor =
                JobExecutor::new(job.into(), options.clone(), sink.clone(), client.clone());
            let minimum = config.delay_ms;
            tasks.spawn(async move {
                let outcome = executor.run().await;
                control
                    .run_durable(0, move || {
                        if mode == Mode::FairTurns {
                            state.resolve_turn(&domain, &outcome, minimum, chrono::Utc::now())
                        } else {
                            state.resolve(&domain, &outcome, minimum, chrono::Utc::now())
                        }
                    })
                    .await??;
                Ok::<_, anyhow::Error>(())
            });
        }
        if tasks.is_empty() {
            let due = frontier
                .next_eligible_at()?
                .context("unfinished frontier has no eligible site")?;
            let wait = due
                .saturating_sub(chrono::Utc::now().timestamp_millis().max(0) as u64)
                .max(1);
            tokio::time::sleep(Duration::from_millis(wait)).await;
        } else if tasks.len() < config.workers {
            let due = frontier.next_eligible_at()?;
            if let Some(due) = due {
                let wait = due
                    .saturating_sub(chrono::Utc::now().timestamp_millis().max(0) as u64)
                    .max(1);
                tokio::select! {
                    result = tasks.join_next() => { result.unwrap()??; }
                    _ = tokio::time::sleep(Duration::from_millis(wait)) => {}
                }
            } else {
                tasks.join_next().await.unwrap()??;
            }
        } else {
            tasks.join_next().await.unwrap()??;
        }
        anyhow::ensure!(
            started.elapsed() < Duration::from_secs(120),
            "scheduler profile exceeded deadline"
        );
    }
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    sink.spool.lock().unwrap().seal()?;
    ready_lags.sort_unstable();
    let expected = (config.origins * config.pages) as u64;
    anyhow::ensure!(
        metrics.stats().records_durable == expected
            && frontier.metrics.snapshot().commits == expected,
        "scheduler lost useful captures"
    );
    let lag_p99 = ready_lags
        .get((ready_lags.len() * 99 / 100).min(ready_lags.len().saturating_sub(1)))
        .copied()
        .unwrap_or(0);
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    anyhow::ensure!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0,
        "resource measurement failed"
    );
    let usage = unsafe { usage.assume_init() };
    let cpu_ms = cpu_ms()? - cpu_started;
    Ok(
        serde_json::json!({"mode":config.mode,"origins":config.origins,"pages":expected,"duration_ms":duration_ms,
        "pages_per_second":expected as f64 / duration_ms * 1000.0,"cpu_ms":cpu_ms,"rss_peak_bytes":usage.ru_maxrss as u64 * 1024,
        "ready_lag_p99_ms":lag_p99,"ready_lag_max_ms":ready_lags.last().copied().unwrap_or(0),"turns":turns,
        "scheduler":client.robots_txt_manager().scheduler().stats(),"robots":client.robots_txt_manager().stats(),
        "transport":transport.stats(),"pipeline":pipeline.stats(),"frontier":frontier.metrics.snapshot(),"spool":metrics.stats()}),
    )
}

fn cpu_ms() -> Result<f64> {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    anyhow::ensure!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0,
        "CPU measurement failed"
    );
    let usage = unsafe { usage.assume_init() };
    Ok(
        (usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) as f64 * 1000.0
            + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as f64 / 1000.0,
    )
}
