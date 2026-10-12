//! Local-only capacity bank. This module is absent from default production builds.
//! Logical HTTP and HTTPS are both transported as plain HTTP to pinned loopback
//! sockets; it measures the executor, not Internet DNS/TLS or frontier scheduling.
use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    io::Write,
    net::{Ipv4Addr, SocketAddr},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use futures::{stream, StreamExt};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    observer::{Counters, FrontierState, Observer, Snapshot, Warc},
    robot_client::RobotClient,
    runtime::{Dns, Http, IoFuture, Metrics, Runtime, SystemClock},
    CrawlDatum, DatumSink, Domain, Job, JobExecutor, WeightedUrl,
};
use crate::{
    config::{CrawlerConfig, S3Config, UserAgent},
    warc,
    webpage::Html,
};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub host: String,
    pub port: u16,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub origins: Vec<Origin>,
    pub paths: Vec<String>,
    pub observer: bool,
    #[serde(default)]
    pub observer_mode: ObserverMode,
    pub output_dir: PathBuf,
    pub min_crawl_delay_ms: u64,
    pub max_crawl_delay_ms: u64,
    pub workers: usize,
    pub timeout_seconds: u64,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObserverMode {
    #[default]
    Async,
    LegacySync,
}

struct LoopbackDns {
    origins: BTreeMap<String, u16>,
}
impl Dns for LoopbackDns {
    fn resolve<'a>(&'a self, host: &'a str) -> IoFuture<'a, super::Result<SocketAddr>> {
        Box::pin(async move {
            self.origins
                .get(host)
                .map(|port| SocketAddr::from((Ipv4Addr::LOCALHOST, *port)))
                .ok_or_else(|| {
                    anyhow!("destination is outside the local benchmark allowlist: {host}").into()
                })
        })
    }
}

struct LoopbackHttp {
    dns: Arc<dyn Dns>,
    metrics: Arc<Metrics>,
    config: CrawlerConfig,
}
impl Http for LoopbackHttp {
    fn send(
        &self,
        logical: Url,
        timeout: Option<Duration>,
    ) -> IoFuture<'_, super::Result<reqwest::Response>> {
        Box::pin(async move {
            if !matches!(logical.scheme(), "http" | "https")
                || !logical.username().is_empty()
                || logical.password().is_some()
                || logical.port().is_some_and(|p| p != 80 && p != 443)
            {
                self.metrics.blocked();
                return Err(anyhow!("invalid local benchmark destination").into());
            }
            let host = logical.host_str().ok_or_else(|| anyhow!("missing host"))?;
            let address = {
                let _timer = self.metrics.timer("dns");
                match self.dns.resolve(host).await {
                    Ok(address) if address.ip().is_loopback() => address,
                    _ => {
                        self.metrics.blocked();
                        return Err(anyhow!(
                            "benchmark DNS must resolve an allowlisted loopback address"
                        )
                        .into());
                    }
                }
            };
            let mut actual = logical.clone();
            actual
                .set_scheme("http")
                .map_err(|_| anyhow!("invalid scheme"))?;
            actual
                .set_port(Some(address.port()))
                .map_err(|_| anyhow!("invalid port"))?;
            // Disable environment proxies and automatic redirects. Every request,
            // including robots redirects, returns through this allowlist again.
            let client = super::robot_client::client_builder(&self.config)
                .no_proxy()
                .resolve(host, address)
                .build()
                .map_err(|e| anyhow!(e))?;
            let mut request = client.get(actual).header(reqwest::header::HOST, host);
            if let Some(timeout) = timeout {
                request = request.timeout(timeout);
            }
            self.metrics.request(&logical);
            let _timer = self.metrics.timer("http_headers");
            let result = request
                .send()
                .await
                .map_err(|e| super::Error::from(anyhow!(e)));
            if result.is_err() {
                self.metrics.failure();
            }
            result
        })
    }
}

struct Saved {
    path: PathBuf,
    url: String,
    body_hash: md5::Digest,
    payload_type: warc::PayloadType,
    fetch_time_ms: u64,
    date: chrono::DateTime<chrono::Utc>,
    bytes: usize,
}
struct BenchSink {
    directory: PathBuf,
    saved: Mutex<Vec<Saved>>,
    observer: Option<Arc<Observer>>,
    fatal: Mutex<Option<String>>,
}
impl BenchSink {
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
                capture: None,
                fetch_time_ms: datum.fetch_time_ms,
            },
        })?;
        let bytes = writer.finish()?;
        let filename = format!("{}.warc.gz", uuid::Uuid::new_v4());
        let path = self.directory.join(&filename);
        let tmp = path.with_extension("tmp");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&tmp, &path)?;
        fs::File::open(&self.directory)?.sync_all()?;
        self.saved.lock().unwrap().push(Saved {
            path,
            url: datum.url.to_string(),
            body_hash: md5::compute(datum.body.as_bytes()),
            payload_type: datum.payload_type,
            fetch_time_ms: datum.fetch_time_ms,
            date: datum.date,
            bytes: datum.body.len(),
        });
        if let Some(observer) = &self.observer {
            observer.saved(
                &Domain::from(&datum.url),
                datum,
                filename,
                bytes.len() as u64,
            );
        }
        Ok(())
    }
}
impl DatumSink for BenchSink {
    async fn write(&self, datum: CrawlDatum) -> super::Result<()> {
        self.commit(&datum).map_err(|error| {
            *self.fatal.lock().unwrap() = Some(error.to_string());
            super::Error::Anyhow(error)
        })
    }
    async fn finish(&self) -> super::Result<()> {
        if let Some(error) = &*self.fatal.lock().unwrap() {
            return Err(anyhow!("benchmark storage failed: {error}").into());
        }
        Ok(())
    }
}

fn observer(config: &Config, metrics: Arc<Metrics>) -> Result<Option<Arc<Observer>>> {
    if !config.observer {
        return Ok(None);
    }
    let now = chrono::Utc::now().to_rfc3339();
    let path = config.output_dir.join("observer.json");
    let snapshot = Snapshot {
        schema_version: 1,
        run_id: "isolated-benchmark".into(),
        pid: std::process::id(),
        started_at: now.clone(),
        updated_at: now,
        finished_at: None,
        phase: "running".into(),
        user_agent: "CrawlerBench/1.0".into(),
        workers: config.workers,
        min_crawl_delay_ms: config.min_crawl_delay_ms,
        pages_per_domain: config.paths.len() as u64,
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
    };
    let observer = match config.observer_mode {
        ObserverMode::Async => Observer::new(path, snapshot),
        ObserverMode::LegacySync => Observer::new_legacy(path, snapshot),
    }?;
    Ok(Some(Arc::new(observer.with_metrics(metrics))))
}

fn process_io() -> BTreeMap<String, u64> {
    fs::read_to_string("/proc/self/io")
        .unwrap_or_default()
        .lines()
        .filter_map(|line| {
            let (name, value) = line.split_once(':')?;
            Some((name.to_owned(), value.trim().parse::<u64>().ok()?))
        })
        .collect()
}

fn cpu_ticks() -> Option<(u64, u64)> {
    let stat = fs::read_to_string("/proc/self/stat").ok()?;
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    Some((fields.get(11)?.parse().ok()?, fields.get(12)?.parse().ok()?))
}

pub async fn run(config: Config) -> Result<serde_json::Value> {
    anyhow::ensure!(
        !config.origins.is_empty() && !config.paths.is_empty(),
        "origins and paths are required"
    );
    anyhow::ensure!(
        (1..=128).contains(&config.workers),
        "workers must be 1..128"
    );
    anyhow::ensure!(
        config.timeout_seconds > 0
            && config.min_crawl_delay_ms > 0
            && config.max_crawl_delay_ms >= config.min_crawl_delay_ms,
        "invalid delays or timeout"
    );
    let mut domains = BTreeSet::new();
    let mut origins = BTreeMap::new();
    for origin in &config.origins {
        let url = Url::parse(&format!("http://{}/", origin.host))?;
        anyhow::ensure!(
            origin.host.ends_with(".test")
                && url.host_str() == Some(&origin.host)
                && origin.port > 0
                && url.port().is_none(),
            "origins must be distinct .test hostnames with nonzero loopback ports"
        );
        let domain = Domain::from(&url);
        anyhow::ensure!(
            !domain.as_str().is_empty() && domains.insert(domain.as_str().to_owned()),
            "origins must have distinct registrable domains"
        );
        origins.insert(origin.host.clone(), origin.port);
    }
    for path in &config.paths {
        anyhow::ensure!(
            path.starts_with('/') && !path.starts_with("//") && !path.contains('#'),
            "paths must be local absolute paths"
        );
    }
    fs::create_dir_all(&config.output_dir)?;
    let directory = config.output_dir.join("warc");
    fs::create_dir(&directory).context("benchmark requires a fresh output directory")?;
    let crawler_config = Arc::new(CrawlerConfig {
        spool: Default::default(),
        num_worker_threads: config.workers,
        user_agent: UserAgent {
            full: "CrawlerBench/1.0 (isolated local benchmark)".into(),
            token: "CrawlerBench".into(),
        },
        network_policy: crate::config::CrawlNetworkPolicy::PublicOnly,
        transport: super::transport::TransportConfig::default(),
        body: super::body::BodyConfig::default(),
        pipeline: super::pipeline::PipelineConfig::default(),
        max_redirect_hops: crate::config::default_max_redirect_hops(),
        robots_txt_cache_sec: 3600,
        min_politeness_factor: 0,
        start_politeness_factor: 0,
        min_crawl_delay_ms: config.min_crawl_delay_ms,
        max_crawl_delay_ms: config.max_crawl_delay_ms,
        max_politeness_factor: 11,
        max_url_slowdown_retry: 3,
        timeout_seconds: config.timeout_seconds,
        router_hosts: Vec::new(),
        s3: S3Config {
            bucket: String::new(),
            folder: String::new(),
            access_key: String::new(),
            secret_key: String::new(),
            endpoint: String::new(),
        },
    });
    let metrics = Arc::new(Metrics::default());
    let runtime = Arc::new(Runtime {
        http: Arc::new(LoopbackHttp {
            dns: Arc::new(LoopbackDns { origins }),
            metrics: metrics.clone(),
            config: (*crawler_config).clone(),
        }),
        clock: Arc::new(SystemClock),
        metrics: metrics.clone(),
    });
    let client = RobotClient::with_runtime(&crawler_config, runtime)?;
    let observer = observer(&config, metrics.clone())?;
    let sink = Arc::new(BenchSink {
        directory,
        observer: observer.clone(),
        saved: Mutex::new(Vec::new()),
        fatal: Mutex::new(None),
    });
    let mut jobs = Vec::new();
    for origin in &config.origins {
        let base = Url::parse(&format!("http://{}/", origin.host))?;
        let domain = Domain::from(&base);
        if let Some(observer) = &observer {
            observer.register_site(domain.as_str().into(), config.paths.len() as u64);
        }
        let urls = config
            .paths
            .iter()
            .map(|path| {
                Ok(WeightedUrl {
                    url: base.join(path)?,
                    weight: 1.0,
                })
            })
            .collect::<Result<VecDeque<_>>>()?;
        let job_domain = domain.as_str().to_owned();
        let job = Job {
            domain,
            urls,
            wandering_urls: 0,
        };
        let mut executor = JobExecutor::new(
            job.into(),
            crawler_config.clone(),
            sink.clone(),
            client.clone(),
        );
        if let Some(observer) = &observer {
            executor = executor.with_observer(observer.clone());
        }
        jobs.push((job_domain, executor));
    }
    let ticks_per_second = std::process::Command::new("getconf")
        .arg("CLK_TCK")
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .and_then(|out| out.trim().parse::<f64>().ok())
        .filter(|n| *n > 0.0);
    let io_before = process_io();
    let cpu_before = cpu_ticks();
    metrics.reset();
    let started = Instant::now();
    let outcomes = stream::iter(jobs)
        .map(|(domain, executor)| {
            metrics.stage("queue_wait", started.elapsed());
            tokio::spawn(async move { (domain, executor.run().await) })
        })
        .buffer_unordered(config.workers)
        .collect::<Vec<_>>()
        .await;
    let mut job_outcomes = BTreeMap::new();
    for outcome in outcomes {
        let (domain, outcome) = outcome.context("benchmark executor panicked")?;
        anyhow::ensure!(
            !matches!(
                outcome.status,
                super::JobStatus::StorageFailed | super::JobStatus::Cancelled
            ),
            "benchmark job did not finish: {outcome:?}"
        );
        job_outcomes.insert(domain, outcome);
    }
    sink.finish().await?;
    if let Some(observer) = &observer {
        let _ = observer.flush();
    }
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    let cpu_after = cpu_ticks();
    let io = process_io()
        .into_iter()
        .map(|(name, value)| {
            let before = io_before.get(&name).copied().unwrap_or(0);
            (name, value.saturating_sub(before))
        })
        .collect::<BTreeMap<_, _>>();
    let resources_crawl = match (cpu_before, cpu_after, ticks_per_second) {
        (Some((before_user, before_system)), Some((user, system)), Some(rate)) => {
            serde_json::json!({
                "cpu_user_seconds": user.saturating_sub(before_user) as f64 / rate,
                "cpu_system_seconds": system.saturating_sub(before_system) as f64 / rate,
                "clock_ticks_per_second": rate,
            })
        }
        _ => serde_json::Value::Null,
    };
    // Contract verification is deliberately outside the timed crawl: use the
    // same WARC reader and HTML parser consumed downstream by the indexer.
    let saved = sink.saved.lock().unwrap();
    for expected in saved.iter() {
        let file = warc::WarcFile::open(&expected.path)?;
        let records = file.records().collect::<crate::Result<Vec<_>>>()?;
        anyhow::ensure!(records.len() == 1, "expected exactly one WARC record");
        let record = &records[0];
        anyhow::ensure!(
            record.request.url == expected.url
                && md5::compute(record.response.body.as_bytes()) == expected.body_hash
                && record.response.payload_type == Some(expected.payload_type)
                && record.metadata.fetch_time_ms == expected.fetch_time_ms
                && record.request.date == Some(expected.date),
            "WARC roundtrip changed URL, body, payload type, fetch time or date"
        );
        Html::parse(&record.response.body, &record.request.url)
            .map_err(|_| anyhow!("downstream HTML parse failed"))?;
    }
    let mut measured = metrics.snapshot();
    for stage in [
        "dns",
        "http_headers",
        "http_body",
        "politeness",
        "parse",
        "sink",
        "observer",
        "queue_wait",
    ] {
        measured.stages_ms.entry(stage.into()).or_default();
    }
    let mut result = serde_json::to_value(measured)?;
    let object = result.as_object_mut().unwrap();
    object.insert("duration_ms".into(), duration_ms.into());
    object.insert("saved_pages".into(), saved.len().into());
    object.insert(
        "saved_bytes".into(),
        saved.iter().map(|s| s.bytes as u64).sum::<u64>().into(),
    );
    object.insert("warc_files".into(), saved.len().into());
    object.insert("warc_verified".into(), true.into());
    object.insert("resources_io".into(), serde_json::to_value(io)?);
    object.insert("resources_crawl".into(), resources_crawl);
    object.insert("job_outcomes".into(), serde_json::to_value(job_outcomes)?);
    object.insert(
        "observer_mode".into(),
        if config.observer {
            serde_json::to_value(config.observer_mode)?
        } else {
            "off".into()
        },
    );
    if let Some(observer) = &observer {
        object.insert(
            "observer_stats".into(),
            serde_json::to_value(observer.statistics())?,
        );
        let snapshot = fs::read(config.output_dir.join("observer.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
        object.insert(
            "observer_counters".into(),
            snapshot
                .and_then(|snapshot| snapshot.get("counters").cloned())
                .unwrap_or(serde_json::Value::Null),
        );
    }
    object.insert(
        "crawl_policy".into(),
        serde_json::json!({
            "network_policy": crawler_config.network_policy,
            "max_redirect_hops": crawler_config.max_redirect_hops,
            "observer_controls_behavior": false,
        }),
    );
    object.insert("stage_semantics".into(), "cumulative worker wall time; stages overlap across concurrent jobs; sink includes saved observer callback; DNS is injected; no Internet/TLS; WARC verification excluded".into());
    fs::write(
        config.output_dir.join("result.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn injected_dns_is_closed_to_every_unlisted_destination() {
        let dns = LoopbackDns {
            origins: BTreeMap::from([("fixture.test".into(), 12345)]),
        };
        assert_eq!(
            dns.resolve("fixture.test").await.unwrap(),
            "127.0.0.1:12345".parse().unwrap()
        );
        for host in [
            "outside.invalid",
            "localhost",
            "127.0.0.1",
            "fixture.test.evil.invalid",
        ] {
            assert!(dns.resolve(host).await.is_err());
        }
    }

    #[tokio::test]
    async fn injected_http_blocks_redirect_destinations_before_any_request() {
        let metrics = Arc::new(Metrics::default());
        let http = LoopbackHttp {
            dns: Arc::new(LoopbackDns {
                origins: BTreeMap::from([("fixture.test".into(), 12345)]),
            }),
            metrics: metrics.clone(),
            config: CrawlerConfig::for_tests(),
        };
        for target in [
            "https://outside.invalid/escape",
            "http://127.0.0.1/admin",
            "http://fixture.test:9999/escape",
            "http://user:pass@fixture.test/",
        ] {
            assert!(http.send(Url::parse(target).unwrap(), None).await.is_err());
        }
        assert_eq!(metrics.snapshot().client_requests, 0);
        assert_eq!(metrics.snapshot().blocked_outbound, 4);
    }
}
