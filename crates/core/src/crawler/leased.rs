//! Opt-in protocol-v2 workers; the embedded frontier remains the default.
use super::{
    runtime::IoFuture,
    transport::{HttpAuthority, SharedTransport},
    CrawlDatum, DatumSink, Domain, Job, JobExecutor, WeightedUrl,
};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use url::Url;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub endpoint: Url,
    pub secret_file: PathBuf,
    pub node_id: String,
    pub worker_id: String,
    pub crawler: super::local::Config,
    #[serde(default)]
    pub maximum_jobs: Option<usize>,
    #[serde(default)]
    pub experimental: bool,
}

#[derive(Clone)]
struct Rpc {
    serialization: Arc<tokio::sync::Semaphore>,
    client: reqwest::Client,
    endpoint: Url,
    key: Arc<ring::hmac::Key>,
    node: String,
    worker: String,
}
impl Rpc {
    fn new(config: &Config) -> Result<Self> {
        let host = config.endpoint.host_str().unwrap_or("");
        let loopback = host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
        anyhow::ensure!(
            config.endpoint.scheme() == "https" || loopback,
            "cluster RPC requires HTTPS or literal loopback"
        );
        anyhow::ensure!(
            config.endpoint.username().is_empty() && config.endpoint.password().is_none(),
            "RPC URL cannot contain credentials"
        );
        let secret = std::fs::read(&config.secret_file)?;
        let secret = secret.strip_suffix(b"\n").unwrap_or(&secret);
        anyhow::ensure!(secret.len() >= 32, "cluster secret is too short");
        Ok(Self {
            serialization: Arc::new(tokio::sync::Semaphore::new(2)),
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(8))
                .no_proxy()
                .build()?,
            endpoint: config.endpoint.join("/v2/rpc")?,
            key: Arc::new(ring::hmac::Key::new(ring::hmac::HMAC_SHA256, secret)),
            node: config.node_id.clone(),
            worker: config.worker_id.clone(),
        })
    }
    async fn call(&self, mut value: Value) -> Result<Value> {
        value["version"] = json!(2);
        value["command_id"] = json!(uuid::Uuid::new_v4().to_string());
        value["node_id"] = json!(self.node);
        value["worker_id"] = json!(self.worker);
        let _serialization = self.serialization.acquire().await?;
        let bytes = tokio::task::spawn_blocking(move || serde_json::to_vec(&value)).await??;
        anyhow::ensure!(
            bytes.len() <= 10 * 1024 * 1024,
            "RPC message exceeds worker budget"
        );
        let stamp = chrono::Utc::now().timestamp().to_string();
        let mut signature = ring::hmac::Context::with_key(&self.key);
        signature.update(stamp.as_bytes());
        signature.update(b"\n");
        signature.update(&bytes);
        let mut response = self
            .client
            .post(self.endpoint.clone())
            .header("X-Crawler-Time", stamp)
            .header(
                "X-Crawler-Signature",
                crate::capture::hex(signature.sign().as_ref()),
            )
            .body(bytes)
            .send()
            .await?
            .error_for_status()?;
        let mut output = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(
                output.len() + chunk.len() <= 65536,
                "RPC reply exceeds budget"
            );
            output.extend_from_slice(&chunk);
        }
        let result: Value = serde_json::from_slice(&output)?;
        anyhow::ensure!(result["version"] == 2, "unsupported cluster protocol");
        Ok(result)
    }
}

#[derive(Clone)]
struct Authority {
    rpc: Rpc,
    lease: Value,
    deadline: Arc<Mutex<Instant>>,
    cancelled: Arc<AtomicBool>,
}
impl Authority {
    fn request(&self, action: &str) -> Value {
        let mut value = self.lease.clone();
        value["action"] = json!(action);
        value
    }
    fn update(&self, milliseconds: u64, started: Instant) -> Result<()> {
        let remaining = Duration::from_millis(milliseconds)
            .checked_sub(started.elapsed() + Duration::from_millis(250))
            .context("lease reply arrived after its safety deadline")?;
        *self.deadline.lock().unwrap() = Instant::now() + remaining;
        Ok(())
    }
}
impl HttpAuthority for Authority {
    fn check(&self) -> super::Result<()> {
        if self.cancelled.load(Ordering::Acquire)
            || Instant::now() >= *self.deadline.lock().unwrap()
        {
            return Err(anyhow::anyhow!("cluster lease expired; HTTP admission stopped").into());
        }
        Ok(())
    }
    fn authorize<'a>(&'a self, url: &'a Url) -> IoFuture<'a, super::Result<()>> {
        Box::pin(async move {
            self.check()?;
            let mut request = self.request("authorize");
            request["destination_origin"] = json!(url.origin().ascii_serialization());
            let result = self.rpc.call(request).await?;
            if result["error"] == "courtesy_deferred" {
                return Err(super::Error::CrawlDeferred {
                    retry_after_ms: result["retry_after_ms"].as_u64().unwrap_or(1000),
                });
            }
            if result["authorized"] != true {
                return Err(anyhow::anyhow!("cluster HTTP authority rejected: {}", result).into());
            }
            self.check()
        })
    }
}

struct Sink {
    authority: Authority,
    writer: Arc<super::WarcWriter>,
    pipeline: super::pipeline::Pipeline,
    pending: Arc<super::replication_outbox::Pending>,
}
impl Sink {
    async fn confirm(&self, capture: crate::capture::Capture) -> Result<()> {
        let mut request = self.authority.request("commit");
        let fetch_id = capture.metadata.fetch_id.clone();
        anyhow::ensure!(
            capture.payload.len() <= 4 * 1024 * 1024,
            "replicated capture exceeds 4 MiB profile"
        );
        let bytes = capture
            .payload
            .len()
            .saturating_mul(5)
            .saturating_add(512 * 1024 + 65536);
        request = self
            .pipeline
            .control
            .run_durable(bytes, move || -> Result<Value> {
                request["payload_sha256"] = json!(capture.metadata.payload_sha256);
                request["metadata"] = serde_json::to_value(&capture.metadata)?;
                request["payload_hex"] = json!(crate::capture::hex(&capture.payload));
                Ok(request)
            })
            .await??;
        let reply = self.authority.rpc.call(request).await?;
        anyhow::ensure!(
            reply["ack"] == true && reply["replicated"] == true,
            "no replicated capture confirmation: {}",
            reply
        );
        let pending = self.pending.clone();
        self.pipeline.control.run_durable(0, move || pending.remove(&fetch_id)).await??;
        Ok(())
    }
    async fn stage(&self, capture: crate::capture::Capture) -> Result<()> {
        let pending = self.pending.clone();
        self.pipeline.control.run_durable(capture.payload.len() + capture.metadata.memory_bytes(), move || pending.save(&capture)).await??;
        Ok(())
    }
}
impl DatumSink for Sink {
    async fn reserve_url(&self, _url: &Url) -> super::Result<Option<String>> {
        Ok(self.authority.lease["fetch_id"].as_str().map(str::to_owned))
    }
    async fn write(&self, datum: CrawlDatum) -> super::Result<()> {
        let capture = datum
            .capture
            .clone()
            .context("cluster worker requires capture contract")?;
        self.stage(capture.clone()).await?;
        self.writer.write(datum).await?;
        self.confirm(capture).await.map_err(super::Error::from)
    }
    async fn write_prepared(&self, page: super::pipeline::PreparedPage) -> super::Result<()> {
        let capture = page
            .datum
            .capture
            .clone()
            .context("cluster worker requires capture contract")?;
        self.stage(capture.clone()).await?;
        self.writer.write_prepared(page).await?;
        self.confirm(capture).await.map_err(super::Error::from)
    }
    async fn checkpoint(&self) -> super::Result<()> {
        self.writer.checkpoint().await
    }
    async fn finish(&self) -> super::Result<()> {
        self.writer.finish().await
    }
}

pub async fn run(config: Config) -> Result<()> {
    run_inner(config, None).await
}

#[cfg(feature = "integration-fixtures")]
pub async fn run_fixture(config: Config, transport: SharedTransport) -> Result<()> {
    run_inner(config, Some(transport)).await
}

async fn run_inner(config: Config, fixture: Option<SharedTransport>) -> Result<()> {
    anyhow::ensure!(config.experimental, "distributed worker is a protocol-v2 pilot; require experimental = true and an isolated validated profile");
    config.crawler.validate()?;
    anyhow::ensure!(config.crawler.scheduler.mode == super::scheduler::Mode::LongJobs, "protocol-v2 pilot currently requires explicit scheduler.mode = long_jobs; distributed fair turns are not enabled");
    anyhow::ensure!(
        config.crawler.body.html.wire_limit_bytes <= 4 * 1024 * 1024,
        "cluster profile supports at most 4 MiB captures"
    );
    anyhow::ensure!(config.crawler.pipeline.control.max_bytes >= config.crawler.body.html.wire_limit_bytes.saturating_mul(5).saturating_add(512 * 1024 + 65536), "cluster control budget must cover replicated capture serialization");
    anyhow::ensure!(
        config.crawler.timeout_seconds <= 10 && config.crawler.body.total_timeout_ms <= 10000,
        "cluster request timeout must fit the 11 second takeover cooldown"
    );
    let mut settings = config.crawler.crawler_config();
    #[cfg(feature = "integration-fixtures")]
    if fixture.is_some() {
        settings.network_policy = crate::config::CrawlNetworkPolicy::AllowPrivate;
    }
    let options = Arc::new(settings);
    let rpc = Rpc::new(&config)?;
    let pending = Arc::new(super::replication_outbox::Pending::open(&options.spool.directory)?);
    let resources = super::resources::Controller::new(config.crawler.resources.clone(), options.spool.directory.clone(), 1)?;
    let reconciliation = super::pipeline::Pipeline::new(options.pipeline.clone())?;
    for fetch_id in pending.ids()? {
        let stored = pending.clone();
        let id = fetch_id.clone();
        let capture = reconciliation.control.run_durable(5 * 1024 * 1024, move || stored.load(&id)).await??.context("pending capture disappeared during reconciliation")?;
        let serialization_bytes = capture.payload.len().saturating_mul(5).saturating_add(512 * 1024 + 65536);
        let request = reconciliation.control.run_durable(serialization_bytes, move || {
            json!({"action":"commit","url_id":crate::capture::sha256(capture.metadata.requested_url.as_bytes()),"epoch":0,"token":uuid::Uuid::nil(),"payload_sha256":capture.metadata.payload_sha256,"payload_hex":crate::capture::hex(&capture.payload),"metadata":capture.metadata})
        }).await?;
        let reply = rpc.call(request).await?;
        if reply["ack"] == true && reply["replicated"] == true {
            let stored = pending.clone();
            reconciliation.control.run_durable(0, move || stored.remove(&fetch_id)).await??;
        } else {
            anyhow::ensure!(reply["error"] == "lease_fenced", "pending reconciliation unavailable: {}", reply);
        }
    }
    reconciliation.shutdown().await;
    let spool_config = options.spool.clone();
    let spool_metrics = super::spool::SpoolMetrics::default();
    let observed = spool_metrics.clone();
    let mut spool_stats = tokio::task::spawn_blocking(move || -> Result<_> {
        let spool = super::spool::Spool::open(spool_config, None, observed.clone())?;
        drop(spool);
        Ok(observed.stats())
    }).await??;
    let transport = match fixture {
        Some(transport) => transport,
        None => SharedTransport::new(&options, options.transport.clone())?,
    };
    let mut completed = 0usize;
    let mut received_bytes = 0u64;
    let shutdown = super::local::shutdown_signal();
    tokio::pin!(shutdown);
    loop {
        if config
            .maximum_jobs
            .is_some_and(|maximum| completed >= maximum)
        {
            break;
        }
        if resources.sample_spool(received_bytes, &spool_stats)? == 0 {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_secs(2)) => {},
                stopped = &mut shutdown => { stopped?; break; }
            }
            continue;
        }
        let started = Instant::now();
        let result = tokio::select! {
            result = rpc.call(json!({"action":"claim"})) => result?,
            stopped = &mut shutdown => { stopped?; break; }
        };
        anyhow::ensure!(
            result.get("error").is_none(),
            "frontier unavailable: {}",
            result
        );
        if result["lease"].is_null() {
            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(100)) => {},
                stopped = &mut shutdown => { stopped?; break; }
            }
            continue;
        }
        let lease = result["lease"].clone();
        let authority = Authority {
            rpc: rpc.clone(),
            lease: lease.clone(),
            deadline: Arc::new(Mutex::new(Instant::now())),
            cancelled: Arc::new(AtomicBool::new(false)),
        };
        authority.update(
            lease["lease_ms"]
                .as_u64()
                .context("missing lease duration")?,
            started,
        )?;
        let client = super::robot_client::RobotClient::with_transport(
            &options,
            transport
                .clone()
                .with_authority(Arc::new(authority.clone())),
        );
        let body_reader = client.body_reader().clone();
        let writer = Arc::new(
            super::WarcWriter::open(options.spool.clone(), None, client.pipeline().clone(), None)
                .await?,
        );
        let sink = Arc::new(Sink {
            authority: authority.clone(),
            writer,
            pipeline: client.pipeline().clone(),
            pending: pending.clone(),
        });
        let renewing = authority.clone();
        let renew = tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(
                    renewing.lease["lease_ms"].as_u64().unwrap() / 3,
                ))
                .await;
                let started = Instant::now();
                let result = renewing.rpc.call(renewing.request("renew")).await;
                match result {
                    Ok(value) if value["renewed"] == true => {
                        if renewing
                            .update(value["lease_ms"].as_u64().unwrap_or(0), started)
                            .is_ok()
                        {
                            continue;
                        }
                    }
                    _ => {}
                }
                renewing.cancelled.store(true, Ordering::Release);
                break;
            }
        });
        let url = Url::parse(lease["url"].as_str().context("missing lease URL")?)?;
        let job = Job {
            domain: Domain::from(&url),
            urls: VecDeque::from([WeightedUrl { url, weight: 1.0 }]),
            wandering_urls: 0,
        };
        let existing = {
            let pending = pending.clone();
            let fetch_id = lease["fetch_id"].as_str().context("missing fetch id")?.to_owned();
            sink.pipeline.control.run_durable(5 * 1024 * 1024, move || pending.load(&fetch_id)).await??
        };
        let execution = async {
            if let Some(capture) = existing {
                let final_url = capture.metadata.response.url.clone();
                let mime = capture.metadata.response.values("content-type").next().unwrap_or_default().to_ascii_lowercase();
                let payload_type = if mime.starts_with("application/rss+xml") { crate::warc::PayloadType::Rss } else if mime.starts_with("application/atom+xml") { crate::warc::PayloadType::Atom } else { crate::warc::PayloadType::Html };
                let datum = CrawlDatum { url: Url::parse(&final_url)?, payload_type,
                    body: String::new(), fetch_time_ms:capture.metadata.fetch_time_ms, date:capture.metadata.response.received_at,
                    capture:Some(capture) };
                sink.write(datum).await?;
                let mut outcome = super::JobOutcome::default();
                outcome.record(Url::parse(lease["url"].as_str().unwrap())?, super::FetchOutcome::DurableSuccess { final_url:Url::parse(&final_url)? });
                Ok::<_, anyhow::Error>(outcome)
            } else {
                anyhow::ensure!(!pending.full()?, "replication outbox is full; drain pending captures before HTTP");
                Ok(JobExecutor::new(job.into(), options.clone(), sink.clone(), client)
                    .with_cancellation(authority.cancelled.clone()).run().await)
            }
        };
        tokio::pin!(execution);
        let mut draining = false;
        let outcome = tokio::select! {
            outcome = &mut execution => outcome?,
            stopped = &mut shutdown => {
                stopped?;
                draining = true;
                authority.cancelled.store(true, Ordering::Release);
                execution.await?
            }
        };
        // Keep renewing until admitted durable storage has drained.
        sink.finish().await?;
        received_bytes = received_bytes.saturating_add(body_reader.stats().wire_bytes_received);
        spool_stats = sink.writer.metrics.stats();
        resources.sample_spool(received_bytes, &spool_stats)?;
        renew.abort();
        let _ = renew.await;
        if outcome.counters.useful_pages == 0 {
            let mut request = authority.request("nack");
            let target = outcome
                .urls
                .iter()
                .find(|entry| entry.url.as_str() == lease["url"].as_str().unwrap_or(""));
            let attempts = lease["attempts"].as_u64().unwrap_or(1).min(u32::MAX as u64) as u32;
            let remote_minimum = target
                .and_then(|entry| match entry.outcome {
                    super::FetchOutcome::Deferred { retry_after_ms, .. } => retry_after_ms,
                    _ => None,
                })
                .unwrap_or(60000);
            let pending_local = pending.load(lease["fetch_id"].as_str().unwrap())?.is_some();
            let permanent = !pending_local && (attempts >= options.retry.max_attempts
                || target.is_some_and(|entry| {
                    matches!(
                        entry.outcome,
                        super::FetchOutcome::PermanentFailure { .. }
                            | super::FetchOutcome::Skipped { .. }
                    )
                }));
            request["retry_after_ms"] = json!(options.retry.delay(
                attempts,
                remote_minimum,
                lease["url"].as_str().unwrap_or("")
            ));
            request["permanent"] = json!(permanent);
            request["reason"] = json!(serde_json::to_string(&outcome)?);
            let reply = rpc.call(request).await?;
            anyhow::ensure!(
                reply["nacked"] == true,
                "failed to release lease: {}",
                reply
            );
        }
        completed += 1;
        println!("{}", serde_json::to_string(&outcome)?);
        if draining {
            break;
        }
    }
    Ok(())
}
