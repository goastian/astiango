//! C05 acceptance harness: controlled transport plus the production body reader.
//! Retained text copies model consumers; this does not measure HTML parsing or WARC.
use super::{
    body::{AdmittedBody, BodyConfig, BodyKind, BodyLimits, BodyReader},
    runtime::{IoFuture, SystemClock},
    transport::{DnsAnswer, FixturePolicy, SharedTransport, TransportConfig, TransportResolver},
};
use crate::config::CrawlerConfig;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, VecDeque},
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
pub struct Profile {
    wire_limit_bytes: usize,
    decoded_limit_bytes: usize,
    max_reserved_bytes: usize,
    max_inflight_bodies: usize,
    workers: usize,
    rss_limit_bytes: usize,
    connect_timeout_ms: u64,
    header_timeout_ms: u64,
    idle_timeout_ms: u64,
    total_timeout_ms: u64,
    budget_wait_timeout_ms: u64,
}
impl Profile {
    fn body(&self) -> BodyConfig {
        let limit = BodyLimits {
            wire_limit_bytes: self.wire_limit_bytes,
            decoded_limit_bytes: self.decoded_limit_bytes,
        };
        BodyConfig {
            html: limit.clone(),
            feed: limit.clone(),
            sitemap: limit.clone(),
            robots: limit,
            max_inflight_bodies: self.max_inflight_bodies,
            max_reserved_bytes: self.max_reserved_bytes,
            connect_timeout_ms: self.connect_timeout_ms,
            header_timeout_ms: self.header_timeout_ms,
            idle_timeout_ms: self.idle_timeout_ms,
            total_timeout_ms: self.total_timeout_ms,
            budget_wait_timeout_ms: self.budget_wait_timeout_ms,
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    host: String,
    path: String,
    kind: String,
    expected: String,
    decoded_bytes: Option<usize>,
    decoded_md5: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    scenario: String,
    origins: Vec<String>,
    port: u16,
    cases: Vec<Case>,
    profile: Profile,
    output_dir: PathBuf,
    hold_ms: u64,
    consumer_copies: usize,
}
struct LoopbackResolver {
    hosts: BTreeSet<String>,
}
impl TransportResolver for LoopbackResolver {
    fn resolve<'a>(&'a self, host: &'a str) -> IoFuture<'a, super::Result<DnsAnswer>> {
        Box::pin(async move {
            if !self.hosts.contains(host) {
                return Err(anyhow!("unknown body fixture host {host}").into());
            }
            Ok(DnsAnswer {
                addresses: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
                ttl: Duration::from_secs(300),
            })
        })
    }
}
fn descriptors() -> u64 {
    fs::read_dir("/proc/self/fd").map_or(0, |entries| entries.count() as u64)
}
struct Sampler {
    stop: Arc<AtomicBool>,
    peak: Arc<AtomicU64>,
    thread: Option<thread::JoinHandle<()>>,
}
impl Sampler {
    fn new() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(AtomicU64::new(descriptors()));
        let (thread_stop, thread_peak) = (stop.clone(), peak.clone());
        let thread = thread::spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                thread_peak.fetch_max(descriptors(), Ordering::Relaxed);
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
impl Drop for Sampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[derive(Serialize)]
struct Outcome {
    host: String,
    path: String,
    kind: String,
    cause: String,
    retryable: bool,
    phase: String,
    status: Option<u16>,
    decoded_bytes: usize,
    decoded_md5: Option<String>,
    duration_ms: f64,
    detail: Option<String>,
}
fn kind(name: &str) -> Result<BodyKind> {
    Ok(match name {
        "html" => BodyKind::Html,
        "feed" => BodyKind::Feed,
        "sitemap" => BodyKind::Sitemap,
        "robots" => BodyKind::Robots,
        _ => return Err(anyhow!("invalid fixture body kind {name}")),
    })
}
async fn request(
    transport: SharedTransport,
    reader: BodyReader,
    case: Case,
    port: u16,
    cancel: bool,
) -> Result<(Outcome, Option<AdmittedBody>)> {
    let started = Instant::now();
    let mut outcome = Outcome {
        host: case.host.clone(),
        path: case.path.clone(),
        kind: case.kind.clone(),
        cause: String::new(),
        retryable: false,
        phase: "headers".into(),
        status: None,
        decoded_bytes: 0,
        decoded_md5: None,
        duration_ms: 0.0,
        detail: None,
    };
    let url = Url::parse(&format!("http://{}:{}{}", case.host, port, case.path))?;
    let response = match transport.send(url, None).await {
        Ok(response) => response,
        Err(error) => {
            if let super::Error::Body(error) = &error {
                outcome.cause = error.reason().into();
                outcome.retryable = error.is_retryable();
            } else {
                outcome.cause = "transport_failure".into();
            }
            outcome.detail = Some(error.to_string());
            outcome.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
            return Ok((outcome, None));
        }
    };
    outcome.status = Some(response.status().as_u16());
    outcome.phase = "body".into();
    let future = reader.read(response, kind(&case.kind)?);
    let result = if cancel {
        match tokio::time::timeout(Duration::from_millis(50), future).await {
            Ok(result) => Some(result),
            Err(_) => None,
        }
    } else {
        Some(future.await)
    };
    let body = match result {
        Some(Ok(body)) => {
            outcome.cause = "admitted".into();
            outcome.decoded_bytes = body.len();
            outcome.decoded_md5 = Some(format!("{:x}", md5::compute(body.as_bytes())));
            Some(body)
        }
        Some(Err(error)) => {
            outcome.cause = error.reason().into();
            outcome.retryable = error.is_retryable();
            outcome.detail = Some(error.to_string());
            None
        }
        None => {
            outcome.cause = "cancelled".into();
            None
        }
    };
    outcome.duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((outcome, body))
}
struct Retained {
    _body: AdmittedBody,
    _copies: Vec<String>,
}

pub async fn run(config: Config) -> Result<serde_json::Value> {
    anyhow::ensure!(
        config.port > 0 && (1..=8).contains(&config.origins.len()),
        "invalid loopback fixture origins"
    );
    anyhow::ensure!(
        config.profile.workers == 8 && config.consumer_copies <= 2 && config.hold_ms <= 1000,
        "invalid benchmark limits"
    );
    let hosts: BTreeSet<_> = config.origins.iter().cloned().collect();
    anyhow::ensure!(
        hosts.len() == config.origins.len()
            && hosts.iter().all(|host| host.ends_with(".body.test")
                && host
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))),
        "only controlled *.body.test hosts are permitted"
    );
    for case in &config.cases {
        anyhow::ensure!(
            hosts.contains(&case.host)
                && case.path.starts_with('/')
                && !case.path.starts_with("//"),
            "invalid controlled URL"
        );
        kind(&case.kind)?;
    }
    fs::create_dir_all(&config.output_dir)?;
    let options = config.profile.body();
    options.validate()?;
    let reader = BodyReader::new(options.clone())?;
    let crawler: CrawlerConfig = serde_json::from_value(serde_json::json!({
        "num_worker_threads":8,"user_agent":{"full":"AstianCrawler-C05/1.0 (controlled loopback fixture)","token":"AstianCrawler-C05"},
        "network_policy":"public_only","timeout_seconds":10,"body":options,
        "s3":{"bucket":"","folder":"","access_key":"","secret_key":"","endpoint":""},"router_hosts":[]
    }))?;
    let fd_baseline = descriptors();
    let sampler = Sampler::new();
    let transport = SharedTransport::with_fixture_resolver(
        &crawler,
        TransportConfig {
            max_client_slots: 16,
            max_slots_per_origin: 2,
            dns_positive_ttl_seconds: 300,
            ..TransportConfig::default()
        },
        Arc::new(LoopbackResolver {
            hosts: hosts.clone(),
        }),
        Arc::new(SystemClock),
        FixturePolicy {
            allowed_hosts: hosts,
            certificate_pem: None,
        },
    )?;
    let started = Instant::now();
    let mut outcomes = Vec::new();
    let mut retained_stats = serde_json::Value::Null;
    let mut held_peak = 0;
    if config.scenario == "retained_near_cap" {
        anyhow::ensure!(
            config.cases.len() == 8,
            "retention profile requires eight independent tasks"
        );
        let mut tasks = tokio::task::JoinSet::new();
        for case in &config.cases {
            tasks.spawn(request(
                transport.clone(),
                reader.clone(),
                case.clone(),
                config.port,
                false,
            ));
        }
        let reservation = config.profile.wire_limit_bytes + 3 * config.profile.decoded_limit_bytes;
        let capacity = (config.profile.max_reserved_bytes / reservation)
            .min(config.profile.max_inflight_bodies);
        let mut held = VecDeque::new();
        while let Some(result) = tasks.join_next().await {
            let (outcome, body) = result??;
            outcomes.push(outcome);
            if let Some(body) = body {
                let copies = (0..config.consumer_copies)
                    .map(|_| body.as_str().to_owned())
                    .collect();
                held.push_back(Retained {
                    _body: body,
                    _copies: copies,
                });
                held_peak = held_peak.max(held.len());
                if held.len() >= capacity {
                    retained_stats = serde_json::to_value(reader.stats())?;
                    tokio::time::sleep(Duration::from_millis(config.hold_ms)).await;
                    held.pop_front();
                }
            }
        }
        drop(held);
    } else {
        for case in &config.cases {
            let (outcome, body) = request(
                transport.clone(),
                reader.clone(),
                case.clone(),
                config.port,
                config.scenario == "cancel_body",
            )
            .await?;
            outcomes.push(outcome);
            drop(body);
        }
    }
    outcomes.sort_by(|left, right| (&left.host, &left.path).cmp(&(&right.host, &right.path)));
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    let mut result = serde_json::json!({"scenario":config.scenario,"configuration":config.profile,
        "requests":outcomes.len(),"outcomes":outcomes,"duration_ms":duration_ms,
        "body_stats":reader.stats(),"transport_stats":transport.stats(),"retained_stats":retained_stats,
        "held_bodies_peak":held_peak,"consumer_copies":config.consumer_copies,"hold_ms":config.hold_ms,
        "fd_baseline":fd_baseline,"scope":"production transport/body admission/decoding; retained consumer copies; no parser or WARC"});
    drop(transport);
    drop(reader);
    tokio::time::sleep(Duration::from_millis(200)).await;
    result["fd_peak"] = sampler.peak.load(Ordering::Relaxed).into();
    drop(sampler);
    result["fd_final"] = descriptors().into();
    fs::write(
        config.output_dir.join("result.json"),
        serde_json::to_vec_pretty(&result)?,
    )?;
    Ok(result)
}
