use super::{
    body::{BodyConfig, BodyKind},
    extraction::VisitedUrls,
    observer::{Counters, FrontierState, Observer, Snapshot, Warc},
    pipeline::{Pipeline, PipelineConfig},
    robot_client::RobotClient,
    runtime::{IoFuture, SystemClock},
    transport::{DnsAnswer, FixturePolicy, SharedTransport, TransportResolver},
    CrawlDatum, DatumSink, Domain,
};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
use url::Url;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub host: String,
    pub path: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub mode: String,
    pub scenario: String,
    pub runtime_threads: usize,
    pub origins: BTreeSet<String>,
    pub port: u16,
    pub cases: Vec<Case>,
    pub body: BodyConfig,
    pub pipeline: PipelineConfig,
    pub async_jobs: usize,
    pub parse_delay_ms: u64,
    pub storage_delay_ms: u64,
    pub rss_limit_bytes: u64,
    pub output_dir: PathBuf,
}
struct Resolver(BTreeSet<String>);
impl TransportResolver for Resolver {
    fn resolve<'a>(&'a self, host: &'a str) -> IoFuture<'a, super::Result<DnsAnswer>> {
        Box::pin(async move {
            if !self.0.contains(host) {
                return Err(anyhow!("unknown pipeline fixture host").into());
            }
            Ok(DnsAnswer {
                addresses: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
                ttl: Duration::from_secs(300),
            })
        })
    }
}
#[derive(Serialize)]
pub struct Measurement {
    pub scenario: String,
    pub mode: String,
    pub runtime_threads: usize,
    pub pages: usize,
    pub duration_ms: f64,
    pub pages_per_second: f64,
    pub cpu_ms_per_page: f64,
    pub rss_peak_bytes: u64,
    pub rss_limit_bytes: u64,
    pub fds_peak: u64,
    pub controller_delay_ms: f64,
    pub pipeline: super::pipeline::PipelineStats,
    pub body: super::body::BodyStats,
    pub transport: super::transport::TransportStats,
    pub archive_payload_md5: BTreeMap<String, String>,
    pub extraction: super::extraction::ExtractionStats,
}
fn usage() -> (u64, f64) {
    // SAFETY: zero is valid for the C rusage output structure.
    let mut value: libc::rusage = unsafe { std::mem::zeroed() };
    // SAFETY: value is a writable, correctly aligned rusage structure.
    assert_eq!(unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut value) }, 0);
    let cpu = (value.ru_utime.tv_sec + value.ru_stime.tv_sec) as f64 * 1000.
        + (value.ru_utime.tv_usec + value.ru_stime.tv_usec) as f64 / 1000.;
    (value.ru_maxrss as u64 * 1024, cpu)
}
pub async fn run(config: Config) -> Result<Measurement> {
    anyhow::ensure!(
        matches!(config.mode.as_str(), "legacy" | "bounded")
            && (1..=4).contains(&config.runtime_threads),
        "invalid pipeline benchmark mode/profile"
    );
    anyhow::ensure!(
        (1..=128).contains(&config.async_jobs)
            && !config.cases.is_empty()
            && config.output_dir.is_absolute(),
        "invalid local pipeline benchmark input"
    );
    config.pipeline.validate_body(&config.body)?;
    fs::create_dir_all(&config.output_dir)?;
    let mut pipeline = Pipeline::new(config.pipeline.clone())?;
    pipeline.fixture_mode(
        config.mode == "legacy",
        config.parse_delay_ms,
        config.storage_delay_ms,
    );
    let crawler: crate::config::CrawlerConfig = serde_json::from_value(serde_json::json!({
        "num_worker_threads": config.async_jobs, "user_agent": {"full":"CrawlerPipelineBench/1.0", "token":"CrawlerPipelineBench"},
        "network_policy":"allow_private", "body":config.body, "pipeline":config.pipeline,
        "timeout_seconds":30, "s3":{"access_key":"","secret_key":"","bucket":"","folder":"","endpoint":""}, "router_hosts":[]
    }))?;
    let transport = SharedTransport::with_fixture_resolver(
        &crawler,
        crawler.transport.clone(),
        Arc::new(Resolver(config.origins.clone())),
        Arc::new(SystemClock),
        FixturePolicy {
            allowed_hosts: config.origins.clone(),
            certificate_pem: None,
        },
    )?;
    let client = RobotClient::with_transport(&crawler, transport).with_pipeline(pipeline.clone());
    let now = chrono::Utc::now().to_rfc3339();
    let observer = Arc::new(Observer::new(
        config.output_dir.join("status.json"),
        Snapshot {
            schema_version: 1,
            run_id: "isolated-c07-benchmark".into(),
            pid: std::process::id(),
            updated_at: now.clone(),
            started_at: now,
            finished_at: None,
            phase: "running".into(),
            user_agent: "CrawlerPipelineBench".into(),
            workers: config.async_jobs,
            min_crawl_delay_ms: 0,
            pages_per_domain: 1,
            continuous: false,
            frontier: FrontierState::default(),
            counters: Counters::default(),
            sites: Vec::new(),
            recent_events: Vec::new(),
            history: Vec::new(),
            warc: Warc {
                directory: config.output_dir.join("warc").display().to_string(),
                files: 0,
                bytes: 0,
                current_file: None,
            },
        },
    )?);
    let sink = super::local::benchmark_sink(
        config.output_dir.join("warc"),
        observer.clone(),
        client.extractor().clone(),
        pipeline.clone(),
    )?;
    let stop = Arc::new(AtomicBool::new(false));
    let fds = Arc::new(AtomicU64::new(0));
    let sampler = {
        let stop = stop.clone();
        let fds = fds.clone();
        thread::spawn(move || {
            while !stop.load(Ordering::Acquire) {
                if let Ok(entries) = fs::read_dir("/proc/self/fd") {
                    fds.fetch_max(entries.count() as u64, Ordering::Relaxed);
                }
                thread::sleep(Duration::from_millis(2));
            }
        })
    };
    pipeline.start_monitor();
    tokio::time::sleep(Duration::from_millis(50)).await;
    let started = Instant::now();
    let cpu_before = usage().1;
    let expected = tokio::time::Instant::now() + Duration::from_millis(100);
    let control = {
        let pipeline = pipeline.clone();
        tokio::spawn(async move {
            tokio::time::sleep_until(expected).await;
            pipeline.control.run_durable(1, || ()).await?;
            Ok::<_, super::pipeline::PipelineError>(
                tokio::time::Instant::now()
                    .saturating_duration_since(expected)
                    .as_secs_f64()
                    * 1000.,
            )
        })
    };
    let mut cases = config.cases.iter().cloned();
    let mut jobs = tokio::task::JoinSet::new();
    loop {
        while jobs.len() < config.async_jobs {
            let Some(case) = cases.next() else { break };
            let client = client.clone();
            let sink = sink.clone();
            let port = config.port;
            jobs.spawn(async move {
                anyhow::ensure!(
                    case.path.starts_with('/') && !case.path.starts_with("//"),
                    "invalid fixture path"
                );
                let url = Url::parse(&format!("http://{}:{}{}", case.host, port, case.path))?;
                let admission = client
                    .pipeline()
                    .admit(client.page_reservation_bytes())
                    .await?;
                let response = client.transport().send(url.clone(), None).await?;
                let body = client.body(response, BodyKind::Page).await?;
                let payload_type = body
                    .payload_type()
                    .ok_or_else(|| anyhow!("missing admitted page MIME"))?;
                let (body, body_lease) = body.into_parts();
                let datum = CrawlDatum {
                    url: url.clone(),
                    body,
                    payload_type,
                    fetch_time_ms: 0,
                    date: chrono::Utc::now(),
                };
                let page = client
                    .pipeline()
                    .prepare_page(
                        datum,
                        body_lease,
                        admission,
                        client.extractor().clone(),
                        Domain::from(&url),
                        Arc::new(VisitedUrls::default()),
                        url,
                    )
                    .await?;
                sink.write_prepared(page).await?;
                Ok::<_, anyhow::Error>(())
            });
        }
        let Some(job) = jobs.join_next().await else {
            break;
        };
        job??;
    }
    sink.finish().await?;
    let duration_ms = started.elapsed().as_secs_f64() * 1000.;
    let cpu_ms = usage().1 - cpu_before;
    let controller_delay_ms = control.await??;
    tokio::time::sleep(Duration::from_millis(30)).await;
    pipeline.shutdown().await;
    observer.finish("completed");
    stop.store(true, Ordering::Release);
    sampler.join().unwrap();
    let mut archived = BTreeMap::new();
    for path in fs::read_dir(config.output_dir.join("warc"))? {
        let file = crate::warc::WarcFile::open(path?.path())?;
        for record in file.records() {
            let record = record?;
            anyhow::ensure!(
                archived
                    .insert(
                        record.request.url,
                        format!("{:x}", md5::compute(record.response.body.as_bytes()))
                    )
                    .is_none(),
                "duplicate archive URL"
            );
        }
    }
    anyhow::ensure!(
        archived.len() == config.cases.len(),
        "each fixture must be durably archived"
    );
    let pages = archived.len();
    Ok(Measurement {
        scenario: config.scenario,
        mode: config.mode,
        runtime_threads: config.runtime_threads,
        pages,
        duration_ms,
        pages_per_second: pages as f64 * 1000. / duration_ms,
        cpu_ms_per_page: cpu_ms / pages as f64,
        rss_peak_bytes: usage().0,
        rss_limit_bytes: config.rss_limit_bytes,
        fds_peak: fds.load(Ordering::Relaxed),
        controller_delay_ms,
        pipeline: pipeline.stats(),
        body: client.body_reader().stats(),
        transport: client.transport().stats(),
        archive_payload_md5: archived,
        extraction: client.extractor().stats(),
    })
}
