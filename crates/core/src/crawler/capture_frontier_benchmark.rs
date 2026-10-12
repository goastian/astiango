use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};
use url::Url;

use super::{
    frontier::Frontier,
    spool::{Spool, SpoolConfig},
    CrawlDatum, Domain, Job, WeightedUrl,
};

#[derive(Deserialize)]
pub struct Config {
    pub action: String,
    pub directory: PathBuf,
    #[serde(default)]
    pub mode: String,
    #[serde(default)]
    pub urls: usize,
    #[serde(default)]
    pub phase: String,
    pub http_base: Option<String>,
}

fn cpu_ms() -> f64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return 0.0;
    }
    let usage = unsafe { usage.assume_init() };
    (usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) as f64 * 1000.0
        + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as f64 / 1000.0
}

fn rss() -> (u64, u64) {
    let current = fs::read_to_string("/proc/self/status")
        .unwrap_or_default()
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")
                .and_then(|v| v.split_whitespace().next())
                .and_then(|v| v.parse::<u64>().ok())
        })
        .unwrap_or(0)
        * 1024;
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    let peak = if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
        unsafe { usage.assume_init() }.ru_maxrss.max(0) as u64 * 1024
    } else {
        current
    };
    (current, peak)
}

fn url(index: usize) -> Url {
    Url::parse(&format!(
        "https://site{:03}-frontier.test/page/{index:09}?variant=original&utm_source=keep",
        index % 128
    ))
    .unwrap()
}

#[derive(Serialize)]
struct Legacy {
    pending: VecDeque<Job>,
    active: BTreeMap<String, Job>,
    seen: HashSet<String>,
}

fn snapshot(path: &Path, value: &Legacy) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    let temporary = path.with_extension("tmp");
    let mut file = fs::File::create(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    fs::File::open(path.parent().unwrap())?.sync_all()?;
    Ok(())
}

fn scale(config: &Config) -> Result<Value> {
    anyhow::ensure!(
        config.urls >= 1000
            && config.urls <= 1_000_000
            && matches!(config.mode.as_str(), "redb" | "json"),
        "invalid scale fixture"
    );
    fs::create_dir_all(&config.directory)?;
    let started = Instant::now();
    let cpu = cpu_ms();
    let (build_ms, update_ms, update_cpu_ms, state, bytes) = if config.mode == "redb" {
        let frontier =
            Frontier::open_database(config.directory.join("frontier.json"), 256, 10_000)?;
        frontier.admit_urls((0..config.urls).map(url))?;
        anyhow::ensure!(
            frontier.metrics.snapshot().urls == config.urls as u64,
            "scale fixture lost admitted URLs"
        );
        let build_ms = started.elapsed().as_secs_f64() * 1000.0;
        let update = Instant::now();
        let update_cpu = cpu_ms();
        frontier.admit_urls((config.urls..config.urls + 128).map(url))?;
        (
            build_ms,
            update.elapsed().as_secs_f64() * 1000.0,
            cpu_ms() - update_cpu,
            serde_json::to_value(frontier.metrics.snapshot())?,
            fs::metadata(&frontier.path)?.len(),
        )
    } else {
        let mut jobs: Vec<Job> = (0..128)
            .map(|i| Job {
                domain: Domain::from(url(i)),
                urls: VecDeque::new(),
                wandering_urls: 0,
            })
            .collect();
        for index in 0..config.urls {
            jobs[index % 128].urls.push_back(WeightedUrl {
                url: url(index),
                weight: 1.0,
            });
        }
        let mut legacy = Legacy {
            seen: jobs.iter().map(|j| j.domain.as_str().to_owned()).collect(),
            pending: jobs.into(),
            active: BTreeMap::new(),
        };
        let path = config.directory.join("frontier.json");
        snapshot(&path, &legacy)?;
        let build_ms = started.elapsed().as_secs_f64() * 1000.0;
        for index in config.urls..config.urls + 128 {
            legacy.pending[index % 128].urls.push_back(WeightedUrl {
                url: url(index),
                weight: 1.0,
            });
        }
        let update = Instant::now();
        let update_cpu = cpu_ms();
        snapshot(&path, &legacy)?;
        (
            build_ms,
            update.elapsed().as_secs_f64() * 1000.0,
            cpu_ms() - update_cpu,
            json!({"urls":config.urls + 128, "cache_bytes":null,"batch_size":128}),
            fs::metadata(path)?.len(),
        )
    };
    let (current, peak) = rss();
    Ok(
        json!({"mode":config.mode,"urls":config.urls,"build_ms":build_ms,"update_ms":update_ms,"update_cpu_ms":update_cpu_ms,
        "total_cpu_ms":cpu_ms()-cpu,"rss_bytes":current,"peak_rss_bytes":peak,"database_bytes":bytes,"state":state}),
    )
}

pub(super) fn barrier(phase: &str) -> ! {
    println!("{}", json!({"barrier":phase}));
    std::io::stdout().flush().unwrap();
    loop {
        std::thread::park();
    }
}

fn sample(url: &Url, fetch_id: String) -> CrawlDatum {
    let mut capture = crate::capture::fixture(
        url,
        fetch_id,
        b"<html><title>Fixture</title><body>original</body></html>".to_vec(),
        vec![crate::capture::Header {
            name: "content-type".into(),
            value_hex: crate::capture::hex(b"text/html; charset=UTF-8"),
        }],
    );
    capture
        .metadata
        .internal_urls
        .push(Url::parse("https://site-frontier.test/discovered").unwrap());
    capture
        .metadata
        .discovery_urls
        .push(Url::parse("https://external-frontier.test/discovered").unwrap());
    CrawlDatum {
        url: url.clone(),
        body: "<html><title>Fixture</title><body>original</body></html>".into(),
        payload_type: crate::warc::PayloadType::Html,
        fetch_time_ms: 1,
        date: capture.metadata.response.received_at,
        capture: Some(capture),
    }
}

fn crash(config: &Config) -> Result<Value> {
    let frontier = Frontier::open_database(config.directory.join("frontier.json"), 256, 10)?;
    let url = Url::parse("https://site-frontier.test/page?utm_source=keep")?;
    frontier.discover([url.clone()])?;
    if config.phase == "admission" {
        barrier(&config.phase);
    }
    frontier.claim()?;
    let id = frontier
        .begin_url(&url)?
        .context("fixture URL already durable")?;
    if config.phase == "reservation" {
        barrier(&config.phase);
    }
    let datum = sample(&url, id);
    let mut spool = Spool::open(
        SpoolConfig {
            directory: config.directory.join("spool"),
            ..Default::default()
        },
        None,
        Default::default(),
    )?;
    if matches!(
        config.phase.as_str(),
        "append_after" | "data_sync_after" | "journal_sync_after" | "seal_after"
    ) {
        spool.fault = Some((config.phase.clone(), true, 0));
    }
    let receipt = spool.append(&[&datum])?.remove(0);
    if config.phase == "archive" {
        barrier(&config.phase);
    }
    frontier.commit_capture(&datum.capture.as_ref().unwrap().metadata, &receipt, false)?;
    if config.phase == "commit" {
        barrier(&config.phase);
    }
    frontier.drain_outbox()?;
    if config.phase == "outbox" {
        barrier(&config.phase);
    }
    spool.seal()?;
    barrier(&config.phase)
}

fn recover(config: &Config) -> Result<Value> {
    let frontier = Frontier::open_database(config.directory.join("frontier.json"), 256, 10)?;
    let url = Url::parse("https://site-frontier.test/page?utm_source=keep")?;
    let settings = SpoolConfig {
        directory: config.directory.join("spool"),
        ..Default::default()
    };
    let mut spool = Spool::open(settings.clone(), None, Default::default())?;
    frontier.reconcile_spool(&settings.directory)?;
    let repeated = if let Some(id) = frontier.begin_url(&url)? {
        let datum = sample(&url, id);
        let receipt = spool.append(&[&datum])?.remove(0);
        frontier.commit_capture(&datum.capture.as_ref().unwrap().metadata, &receipt, false)?;
        frontier.drain_outbox()?;
        true
    } else {
        false
    };
    spool.seal()?;
    let mut ids = HashSet::new();
    let mut records = 0;
    for path in fs::read_dir(settings.directory.join("segments"))? {
        let path = path?.path();
        if path.extension().and_then(|s| s.to_str()) != Some("gz") {
            continue;
        }
        for record in crate::warc::RecordIterator::new(fs::File::open(path)?) {
            let capture = record?
                .metadata
                .capture
                .context("recovery fixture has no capture metadata")?;
            anyhow::ensure!(
                ids.insert(capture.fetch_id),
                "duplicate final capture effect"
            );
            records += 1;
        }
    }
    anyhow::ensure!(
        records == 1
            && frontier.metrics.snapshot().commits == 1
            && frontier.metrics.snapshot().urls == 3
            && frontier.metrics.snapshot().outbox_entries == 0,
        "recovery did not restore capture and discoveries"
    );
    Ok(
        json!({"records":records,"fetch_ids":ids,"state":frontier.metrics.snapshot(),"repeated_fetch":repeated,"spool":spool.metrics.stats()}),
    )
}

struct Resolver;
impl super::transport::TransportResolver for Resolver {
    fn resolve<'a>(
        &'a self,
        _: &'a str,
    ) -> super::runtime::IoFuture<'a, super::Result<super::transport::DnsAnswer>> {
        Box::pin(async {
            Ok(super::transport::DnsAnswer {
                addresses: vec!["127.0.0.1".parse().unwrap()],
                ttl: std::time::Duration::from_secs(60),
            })
        })
    }
}

async fn http(config: &Config) -> Result<Value> {
    let base = Url::parse(
        config
            .http_base
            .as_deref()
            .context("missing fixture base")?,
    )?;
    anyhow::ensure!(
        base.scheme() == "http" && base.host_str() == Some("127.0.0.1"),
        "capture fixture must use loopback"
    );
    let mut options = crate::config::CrawlerConfig::for_tests();
    options.user_agent.token = crate::capture::DEFAULT_AGENT.into();
    let transport = super::transport::SharedTransport::with_fixture_resolver(
        &options,
        Default::default(),
        Arc::new(Resolver),
        Arc::new(super::runtime::SystemClock),
        super::transport::FixturePolicy {
            allowed_hosts: ["capture.test".into(), "redirect.test".into()].into(),
            certificate_pem: None,
        },
    )?;
    let client = super::robot_client::RobotClient::with_transport(&options, transport.clone());
    let frontier = Arc::new(Frontier::open_database(
        config.directory.join("frontier.json"),
        256,
        100,
    )?);
    let hook_frontier = frontier.clone();
    let spool = Arc::new(
        super::warc_writer::WarcWriter::open(
            SpoolConfig {
                directory: config.directory.join("spool"),
                ..Default::default()
            },
            None,
            client.pipeline().clone(),
            Some(Arc::new(move |page, receipt| {
                hook_frontier.commit_capture(
                    &page
                        .datum
                        .capture
                        .as_ref()
                        .context("missing HTTP capture")?
                        .metadata,
                    receipt,
                    false,
                )?;
                hook_frontier.drain_outbox()
            })),
        )
        .await?,
    );
    let mut outcomes = Vec::new();
    for path in [
        "/gzip",
        "/chunked",
        "/latin1",
        "/header-noindex",
        "/meta-agent-none",
        "/scoped-other",
        "/redirect-relative",
        "/redirect-cross",
    ] {
        let requested = Url::parse(&format!(
            "http://capture.test:{}{path}",
            base.port().context("missing fixture port")?
        ))?;
        frontier.admit_urls([requested.clone()])?;
        let id = frontier
            .begin_url(&requested)?
            .context("fixture already captured")?;
        let admission = client
            .pipeline()
            .admit(options.body.html.wire_limit_bytes + 3 * options.body.html.decoded_limit_bytes)
            .await?;
        let mut final_url = requested.clone();
        let mut redirects = Vec::new();
        let response = loop {
            let response = transport.send(final_url.clone(), None).await?;
            let captured =
                crate::capture::HttpResponse::received(&response, &final_url, chrono::Utc::now());
            if response.status().is_redirection() {
                anyhow::ensure!(redirects.len() < 10, "fixture redirect loop");
                let location = response
                    .headers()
                    .get("location")
                    .context("missing fixture redirect")?
                    .to_str()?;
                final_url = final_url.join(location)?;
                redirects.push(captured);
                continue;
            }
            break (response, captured);
        };
        let body = client.body(response.0, super::body::BodyKind::Page).await?;
        let (text, payload, charset, limit, body_lease) = body.into_capture_parts();
        let mut capture = crate::capture::fixture(
            &final_url,
            id,
            payload.as_ref().clone(),
            response.1.headers.clone(),
        );
        capture.metadata.requested_url = requested.to_string();
        capture.metadata.transformations = response.1.transformations();
        capture.metadata.response = response.1;
        capture.metadata.redirects = redirects;
        capture.metadata.charset = charset;
        capture.metadata.decoded_limit_bytes = limit;
        capture.metadata.decoded_bytes = text.len();
        let datum = CrawlDatum {
            url: final_url,
            date: capture.metadata.response.received_at,
            body: text,
            capture: Some(capture),
            payload_type: crate::warc::PayloadType::Html,
            fetch_time_ms: 1,
        };
        let page = client
            .pipeline()
            .prepare_page(
                datum,
                body_lease,
                admission,
                client.extractor().clone(),
                Domain::from(&requested),
                Arc::new(Default::default()),
                requested,
            )
            .await?;
        outcomes.push(page.datum.capture.as_ref().unwrap().metadata.clone());
        super::DatumSink::write_prepared(spool.as_ref(), page).await?;
    }
    super::DatumSink::finish(spool.as_ref()).await?;
    client.pipeline().shutdown().await;
    Ok(
        json!({"captures":outcomes,"frontier":frontier.metrics.snapshot(),"spool":spool.metrics.stats(),"transport":transport.stats()}),
    )
}

pub async fn run(config: Config) -> Result<Value> {
    fs::create_dir_all(&config.directory)?;
    match config.action.as_str() {
        "scale" => scale(&config),
        "crash" => crash(&config),
        "recover" => recover(&config),
        "http" => http(&config).await,
        other => anyhow::bail!("unknown capture/frontier benchmark action {other}"),
    }
}
