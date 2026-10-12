//! C04's loopback-only transport bank. It reads complete HTTP bodies but does not
//! parse documents, follow their links, or claim to measure durable WARC storage.
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{anyhow, Context, Result};
use futures::{stream, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    runtime::{IoFuture, SystemClock},
    transport::{DnsAnswer, FixturePolicy, SharedTransport, TransportConfig, TransportResolver},
};
use crate::config::CrawlerConfig;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathSpec {
    path: String,
    status: u16,
    bytes: usize,
    md5: String,
    location: Option<String>,
    page: bool,
}

#[derive(Clone, Copy, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Legacy,
    Pooled,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    origins: Vec<String>,
    port: u16,
    scheme: String,
    mode: Mode,
    paths: Vec<PathSpec>,
    output_dir: PathBuf,
    workers: usize,
    max_connections: usize,
    max_connections_per_origin: usize,
    dns_ttl_seconds: u64,
    min_delay_ms: u64,
    timeout_seconds: u64,
    certificate: Option<PathBuf>,
}

struct LoopbackResolver {
    hosts: BTreeSet<String>,
    calls: AtomicU64,
    ttl: Duration,
}

impl TransportResolver for LoopbackResolver {
    fn resolve<'a>(&'a self, host: &'a str) -> IoFuture<'a, super::Result<DnsAnswer>> {
        Box::pin(async move {
            if !self.hosts.contains(host) {
                return Err(anyhow!("transport benchmark rejects destination {host}").into());
            }
            self.calls.fetch_add(1, Ordering::Relaxed);
            Ok(DnsAnswer {
                addresses: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
                ttl: self.ttl,
            })
        })
    }
}

#[derive(Clone, Serialize)]
struct Received {
    count: u64,
    status: u16,
    bytes: usize,
    md5: String,
}

#[derive(Default)]
struct Ledger {
    requests: u64,
    pages: u64,
    bytes: u64,
    body_mismatches: u64,
    responses: BTreeMap<String, Received>,
}

fn descriptor_count() -> u64 {
    fs::read_dir("/proc/self/fd").map_or(0, |entries| entries.count() as u64)
}

struct DescriptorSampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: Option<thread::JoinHandle<()>>,
}

impl DescriptorSampler {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(descriptor_count()));
        let stop_thread = stop.clone();
        let peak_thread = peak.clone();
        let thread = thread::spawn(move || {
            while !stop_thread.load(Ordering::Acquire) {
                peak_thread.fetch_max(descriptor_count(), Ordering::Relaxed);
                thread::sleep(Duration::from_millis(2));
            }
        });
        Self {
            stop,
            peak,
            thread: Some(thread),
        }
    }
}

impl Drop for DescriptorSampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Run {
    config: Arc<Config>,
    crawler: CrawlerConfig,
    options: TransportConfig,
    resolver: Arc<LoopbackResolver>,
    policy: FixturePolicy,
    pooled: Option<SharedTransport>,
    ledger: Mutex<Ledger>,
    last_legacy_stats: Mutex<serde_json::Value>,
    fd_peak: Arc<AtomicU64>,
}

impl Run {
    fn fresh_transport(&self) -> Result<SharedTransport> {
        SharedTransport::with_fixture_resolver(
            &self.crawler,
            self.options.clone(),
            self.resolver.clone(),
            Arc::new(SystemClock),
            self.policy.clone(),
        )
        .map_err(Into::into)
    }

    async fn request(&self, host: &str, spec: &PathSpec) -> Result<()> {
        let transport = match &self.pooled {
            Some(transport) => transport.clone(),
            None => self.fresh_transport()?,
        };
        let url = Url::parse(&format!(
            "{}://{}:{}{}",
            self.config.scheme, host, self.config.port, spec.path
        ))?;
        let response = transport.send(url.clone(), None).await?;
        self.fd_peak
            .fetch_max(descriptor_count(), Ordering::Relaxed);
        let status = response.status().as_u16();
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let body = response.bytes().await?;
        let hash = format!("{:x}", md5::compute(&body));
        let mismatch = status != spec.status
            || body.len() != spec.bytes
            || hash != spec.md5
            || location != spec.location;
        {
            let mut ledger = self.ledger.lock().unwrap();
            ledger.requests += 1;
            ledger.pages += u64::from(spec.page);
            ledger.bytes += body.len() as u64;
            ledger.body_mismatches += u64::from(mismatch);
            ledger
                .responses
                .entry(format!("{host}{}", spec.path))
                .and_modify(|entry| entry.count += 1)
                .or_insert(Received {
                    count: 1,
                    status,
                    bytes: body.len(),
                    md5: hash,
                });
        }
        anyhow::ensure!(!mismatch, "fixture response changed for {url}");
        if self.pooled.is_none() {
            *self.last_legacy_stats.lock().unwrap() = serde_json::to_value(transport.stats())?;
        }
        Ok(())
    }

    async fn origin(&self, host: String) -> Result<()> {
        for spec in &self.config.paths {
            self.request(&host, spec).await?;
            if let Some(location) = &spec.location {
                let target = self
                    .config
                    .paths
                    .iter()
                    .find(|candidate| &candidate.path == location)
                    .context("redirect target is absent from the controlled workload")?;
                self.request(&host, target).await?;
            }
            tokio::time::sleep(Duration::from_millis(self.config.min_delay_ms)).await;
        }
        Ok(())
    }
}

pub async fn run(config: Config) -> Result<serde_json::Value> {
    anyhow::ensure!(
        matches!(config.scheme.as_str(), "http" | "https"),
        "only HTTP(S) fixtures are supported"
    );
    anyhow::ensure!(
        config.port > 0 && !config.origins.is_empty() && config.origins.len() <= 128,
        "invalid fixture origins"
    );
    anyhow::ensure!(
        config.workers > 0 && config.workers <= 32,
        "invalid fixture worker limit"
    );
    let hosts: BTreeSet<_> = config.origins.iter().cloned().collect();
    anyhow::ensure!(
        hosts.len() == config.origins.len()
            && hosts.iter().all(|host| host.ends_with(".transport.test")
                && host
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))),
        "only unique controlled *.transport.test hosts are permitted"
    );
    anyhow::ensure!(
        config
            .paths
            .iter()
            .all(|spec| spec.path.starts_with('/') && !spec.path.starts_with("//")),
        "invalid fixture paths"
    );
    fs::create_dir_all(&config.output_dir)?;
    let crawler: CrawlerConfig = serde_json::from_value(serde_json::json!({
        "num_worker_threads": config.workers,
        "user_agent": {"full": "AstianCrawler-C04/1.0 (controlled loopback fixture)", "token": "AstianCrawler-C04"},
        "network_policy": "public_only", "timeout_seconds": config.timeout_seconds,
        "s3": {"bucket": "", "folder": "", "access_key": "", "secret_key": "", "endpoint": ""},
        "router_hosts": []
    }))?;
    let options = options(&config)?;
    let resolver = Arc::new(LoopbackResolver {
        hosts: hosts.clone(),
        calls: AtomicU64::new(0),
        ttl: Duration::from_secs(config.dns_ttl_seconds),
    });
    let policy = FixturePolicy {
        allowed_hosts: hosts,
        certificate_pem: config.certificate.as_ref().map(fs::read).transpose()?,
    };
    let fd_baseline = descriptor_count();
    let sampler = DescriptorSampler::start();
    let pooled = match config.mode {
        Mode::Pooled => Some(SharedTransport::with_fixture_resolver(
            &crawler,
            options.clone(),
            resolver.clone(),
            Arc::new(SystemClock),
            policy.clone(),
        )?),
        Mode::Legacy => None,
    };
    let run = Arc::new(Run {
        config: Arc::new(config),
        crawler,
        options,
        resolver,
        policy,
        pooled,
        ledger: Mutex::new(Ledger::default()),
        last_legacy_stats: Mutex::new(serde_json::Value::Null),
        fd_peak: sampler.peak.clone(),
    });
    let started = Instant::now();
    stream::iter(run.config.origins.clone())
        .map(|host| {
            let run = run.clone();
            async move { run.origin(host).await }
        })
        .buffer_unordered(run.config.workers)
        .try_collect::<Vec<_>>()
        .await?;
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    let transport_stats = match &run.pooled {
        Some(transport) => serde_json::to_value(transport.stats())?,
        None => run.last_legacy_stats.lock().unwrap().clone(),
    };
    let dns_queries = run.resolver.calls.load(Ordering::Relaxed);
    let ledger = run.ledger.lock().unwrap();
    let mut result = serde_json::json!({
        "duration_ms": duration_ms, "requests": ledger.requests, "pages": ledger.pages,
        "bytes": ledger.bytes, "body_mismatches": ledger.body_mismatches, "responses": ledger.responses,
        "dns_queries": dns_queries, "dns_queries_per_page": dns_queries as f64 / ledger.pages as f64,
        "transport_stats": transport_stats,
        "transport_stats_scope": if run.pooled.is_some() { "shared_transport" } else { "last_request_transport" },
        "transport_instances": if run.pooled.is_some() { 1 } else { ledger.requests },
        "fd_baseline": fd_baseline,
        "configuration": {"max_connections": run.config.max_connections, "max_connections_per_origin": run.config.max_connections_per_origin,
            "workers": run.config.workers, "dns_ttl_seconds": run.config.dns_ttl_seconds},
        "scope": "production transport with loopback fixture resolver; full bodies, no document parsing/WARC"
    });
    let output = run.config.output_dir.clone();
    drop(ledger);
    drop(run);
    tokio::time::sleep(Duration::from_millis(200)).await;
    result["fd_peak"] = sampler.peak.load(Ordering::Relaxed).into();
    drop(sampler);
    result["fd_final"] = descriptor_count().into();
    fs::write(
        output.join("result.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}

fn options(config: &Config) -> Result<TransportConfig> {
    Ok(TransportConfig {
        max_client_slots: config.max_connections,
        max_slots_per_origin: config.max_connections_per_origin,
        dns_cache_entries: 1024,
        dns_positive_ttl_seconds: config.dns_ttl_seconds,
        dns_negative_ttl_seconds: 5,
        dns_max_inflight: 32,
        idle_timeout_seconds: 30,
    })
}
