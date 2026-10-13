use super::{frontier::Frontier, plan_store::Store, CrawlDatum, DatumSink, JobExecutor};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use url::Url;

#[derive(Deserialize)]
pub struct Config {
    pub action: String,
    pub directory: PathBuf,
    #[serde(default)]
    pub urls: usize,
    #[serde(default)]
    pub mode: String,
    pub http_base: Option<String>,
}
fn cpu_ms() -> f64 {
    let mut r = std::mem::MaybeUninit::<libc::rusage>::uninit();
    assert_eq!(
        unsafe { libc::getrusage(libc::RUSAGE_SELF, r.as_mut_ptr()) },
        0
    );
    let r = unsafe { r.assume_init() };
    (r.ru_utime.tv_sec + r.ru_stime.tv_sec) as f64 * 1000.0
        + (r.ru_utime.tv_usec + r.ru_stime.tv_usec) as f64 / 1000.0
}
fn rss() -> u64 {
    fs::read_to_string("/proc/self/status")
        .unwrap()
        .lines()
        .find_map(|l| {
            l.strip_prefix("VmRSS:")
                .and_then(|l| l.split_whitespace().next()?.parse::<u64>().ok())
        })
        .unwrap()
        * 1024
}
fn planner(config: &Config) -> Result<Value> {
    let store = Store::open(
        &config.directory.join("planner.redb"),
        "resilience-scale-v1",
    )?;
    store.initialize_budgets((0..128).map(|i| (i, config.urls as u64)))?;
    let before = rss();
    let cpu = cpu_ms();
    let start = Instant::now();
    let offset = store.checkpoint("processed")? as usize;
    for from in (offset..config.urls).step_by(128) {
        let records = (from..(from + 128).min(config.urls))
            .map(|i| {
                let url: Url = format!("https://site{:03}.example/page/{i:09}?sig={i}", i % 128)
                    .parse()
                    .unwrap();
                ((i % 1024) as u64, url, (i % 128) as u128)
            })
            .collect::<Vec<_>>();
        store.insert_batch(&records)?;
        store.set_checkpoint("processed", (from + records.len()) as u64)?;
    }
    let mut count = 0;
    let mut max_job = 0;
    let mut max_bytes = 0;
    for id in 0..1024 {
        let group = store.group(id);
        let mut cursor = None;
        while let Some((_, urls, next)) = group.next_job(&cursor, 128, 16_384)? {
            max_job = max_job.max(urls.len());
            max_bytes = max_bytes.max(urls.iter().map(|u| u.as_str().len()).sum::<usize>());
            count += urls.len();
            cursor = Some(next);
        }
    }
    anyhow::ensure!(
        count == config.urls && store.url_count()? == config.urls as u64,
        "planner lost/duplicated candidates"
    );
    Ok(
        json!({"urls": count, "duration_ms":start.elapsed().as_secs_f64()*1000.0,"cpu_ms":cpu_ms()-cpu,"rss_before_bytes":before,"rss_after_bytes":rss(),"cache_bytes":8*1024*1024,"max_job_urls":max_job,"max_job_bytes":max_bytes,"replayed_from":offset,"hosts":store.host_count()?,"tenants":store.tenant_count()?}),
    )
}
fn discovery(config: &Config) -> Result<Value> {
    let frontier = Frontier::open_database(config.directory.join("frontier.redb"), 128, 50)?;
    let source: Url = "https://fixture.example/sitemap.xml".parse()?;
    let mut xml = String::from("<urlset>");
    for i in 0..config.urls {
        xml.push_str(&format!(
            "<url><loc>https://fixture.example/article/{i}?sig={i}</loc></url>"
        ));
    }
    xml.push_str("</urlset>");
    let before = rss();
    let cpu = cpu_ms();
    let start = Instant::now();
    let count = if config.mode == "legacy_vec" {
        crate::sitemap::parse_sitemap(&xml).len() as u64
    } else {
        frontier.ingest_sitemap(&source, &xml)?.candidates
    };
    let result = json!({"urls":count,"duration_ms":start.elapsed().as_secs_f64()*1000.0,"cpu_ms":cpu_ms()-cpu,"rss_before_bytes":before,"rss_after_bytes":rss(),"frontier":frontier.metrics.snapshot()});
    anyhow::ensure!(count == config.urls as u64, "discovery lost URLs");
    Ok(result)
}
fn diversity(config: &Config) -> Result<Value> {
    let store = Store::open(&config.directory.join("planner.redb"), "diversity-v1")?;
    store.initialize_budgets((0..101).map(|i| (i, 1000)))?;
    for from in (0..1000).step_by(128) {
        let records = (from..(from + 128).min(1000))
            .map(|i| {
                (
                    0,
                    format!("https://popular.example/page/{i}").parse().unwrap(),
                    0,
                )
            })
            .collect::<Vec<_>>();
        store.insert_batch(&records)?;
    }
    let tail = (1..=100)
        .map(|i| {
            (
                1,
                format!("https://tail{i}.example/page/1").parse().unwrap(),
                i,
            )
        })
        .collect::<Vec<_>>();
    store.insert_batch(&tail)?;
    let cpu = cpu_ms();
    let start = Instant::now();
    let mut measurements = Vec::new();
    for fraction in [0.0, 0.2] {
        let mut complete = vec![false; 2];
        let mut cursors = vec![None; 2];
        let mut tenants = std::collections::BTreeSet::new();
        let mut selected = 0;
        let mut explored = 0;
        while selected < 100 {
            let index = super::planner::select_group(&complete, selected, fraction)
                .context("diversity fixture exhausted")?;
            match store
                .group(index as u64)
                .next_job(&cursors[index], 1, 16384)?
            {
                Some((tenant, urls, next)) => {
                    cursors[index] = Some(next);
                    selected += urls.len() as u64;
                    explored += u64::from(index == 1);
                    tenants.insert(tenant.as_str().to_string());
                }
                None => complete[index] = true,
            }
        }
        measurements.push(json!({"exploration_fraction":fraction,"urls":selected,"tenants":tenants.len(),"tail_urls":explored}));
    }
    anyhow::ensure!(
        measurements[0]["tenants"] == 1
            && measurements[1]["tenants"] == 21
            && measurements[1]["tail_urls"] == 20,
        "exploration quota lost diversity"
    );
    Ok(
        json!({"urls":100,"duration_ms":start.elapsed().as_secs_f64()*1000.0,"cpu_ms":cpu_ms()-cpu,"rss_after_bytes":rss(),"comparisons":measurements}),
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
                ttl: Duration::from_secs(60),
            })
        })
    }
}
struct Sink {
    frontier: Arc<Frontier>,
    writer: Arc<super::WarcWriter>,
    pipeline: super::pipeline::Pipeline,
}
impl DatumSink for Sink {
    async fn reserve_url(&self, url: &Url) -> super::Result<Option<String>> {
        let disk = self.frontier.clone();
        let url = url.clone();
        Ok(self
            .pipeline
            .control
            .run_durable(0, move || disk.begin_url(&url))
            .await??)
    }
    async fn revalidation_state(
        &self,
        url: &Url,
    ) -> super::Result<Option<super::recrawl::Resource>> {
        let disk = self.frontier.clone();
        let url = url.clone();
        Ok(self
            .pipeline
            .control
            .run(0, move || disk.revalidation_state(&url))
            .await??)
    }
    async fn authorize_request(
        &self,
        url: &Url,
        target: &Url,
        now: chrono::DateTime<chrono::Utc>,
    ) -> super::Result<()> {
        let disk = self.frontier.clone();
        let url = url.clone();
        let target = target.clone();
        self.pipeline
            .control
            .run_durable(0, move || disk.authorize_request(&url, &target, now))
            .await??
    }
    async fn write(&self, datum: CrawlDatum) -> super::Result<()> {
        self.writer.write(datum).await
    }
    async fn write_prepared(&self, page: super::pipeline::PreparedPage) -> super::Result<()> {
        self.writer.write_prepared(page).await
    }
    async fn checkpoint(&self) -> super::Result<()> {
        self.writer.checkpoint().await
    }
    async fn finish(&self) -> super::Result<()> {
        self.writer.finish().await
    }
}
async fn recrawl(config: &Config) -> Result<Value> {
    let base: Url = config
        .http_base
        .as_deref()
        .context("missing HTTP fixture")?
        .parse()?;
    anyhow::ensure!(
        base.scheme() == "http" && base.host_str() == Some("127.0.0.1"),
        "benchmark requires loopback"
    );
    let mut options = crate::config::CrawlerConfig::for_tests();
    options.network_policy = crate::config::CrawlNetworkPolicy::AllowPrivate;
    options.min_crawl_delay_ms = 0;
    options.start_politeness_factor = 0;
    options.min_politeness_factor = 0;
    options.retry.global_attempts_per_minute = 10000;
    options.retry.origin_attempts_per_minute = 10000;
    options.recrawl = super::recrawl::Config {
        enabled: config.mode == "conditional",
        min_interval_ms: 1,
        initial_interval_ms: 1,
        max_interval_ms: 1,
        full_check_every: 20,
    };
    let frontier = Arc::new(Frontier::open_database(
        config.directory.join("frontier.redb"),
        1,
        config.urls as u64,
    )?);
    let trap_test = config.action == "traps";
    if trap_test {
        frontier.configure_discovery(
            &Default::default(),
            &super::traps::Config {
                enabled: config.mode == "protected",
                calendar_family_budget: 3,
                query_family_budget: 3,
                ..Default::default()
            },
        )?;
    }
    frontier.configure_recrawl(&super::recrawl::Config {
        enabled: !trap_test,
        ..options.recrawl.clone()
    })?;
    frontier.configure_retry(&options.retry)?;
    let transport = super::transport::SharedTransport::with_fixture_resolver(
        &options,
        Default::default(),
        Arc::new(Resolver),
        Arc::new(super::runtime::SystemClock),
        super::transport::FixturePolicy {
            allowed_hosts: ["resilience.test".into()].into(),
            certificate_pem: None,
        },
    )?;
    let client = super::robot_client::RobotClient::with_transport(&options, transport.clone());
    let hook = frontier.clone();
    let writer = Arc::new(
        super::WarcWriter::open(
            super::spool::SpoolConfig {
                directory: config.directory.join("spool"),
                ..Default::default()
            },
            None,
            client.pipeline().clone(),
            Some(Arc::new(move |page, receipt| {
                hook.commit_capture(
                    &page
                        .datum
                        .capture
                        .as_ref()
                        .context("missing capture")?
                        .metadata,
                    receipt,
                    false,
                )?;
                hook.drain_outbox()
            })),
        )
        .await?,
    );
    let sink = Arc::new(Sink {
        frontier: frontier.clone(),
        writer: writer.clone(),
        pipeline: client.pipeline().clone(),
    });
    let mut urls = (0..if trap_test { 200 } else { config.urls })
        .map(|i| format!("http://resilience.test:{}/page/{i}", base.port().unwrap()).parse())
        .collect::<std::result::Result<Vec<Url>, _>>()?;
    if trap_test {
        for day in 1..=20 {
            urls.push(
                format!(
                    "http://resilience.test:{}/calendar/2026/10/{day}",
                    base.port().unwrap()
                )
                .parse()?,
            );
        }
        for color in 1..=20 {
            urls.push(
                format!(
                    "http://resilience.test:{}/browse?color={color}&brand=x&size=y&sort=z",
                    base.port().unwrap()
                )
                .parse()?,
            );
        }
    }
    frontier.admit_urls(urls.clone())?;
    let cpu = cpu_ms();
    let start = Instant::now();
    let mut outcomes = Vec::new();
    let mut captures = Vec::new();
    let mut detection = Vec::new();
    for round in 0..if trap_test { 1 } else { 2 } {
        let changed_at = Instant::now();
        if round == 1 {
            tokio::time::sleep(Duration::from_millis(5)).await;
            anyhow::ensure!(
                frontier.requeue_recrawls(chrono::Utc::now())? == config.urls,
                "recrawl did not queue all resources"
            );
        }
        let mut turns = 0;
        while !frontier.is_empty() {
            if let Some(job) = frontier.claim_turn_at(chrono::Utc::now())? {
                let domain = job.domain.clone();
                let url = job.urls.front().unwrap().url.clone();
                let outcome = JobExecutor::new(
                    job.into(),
                    Arc::new(options.clone()),
                    sink.clone(),
                    client.clone(),
                )
                .run()
                .await;
                anyhow::ensure!(
                    outcome.status != super::JobStatus::StorageFailed,
                    "capture storage failed: {:?}",
                    outcome
                );
                frontier.resolve_turn(&domain, &outcome, 0, chrono::Utc::now())?;
                if round == 1
                    && frontier
                        .revalidation_state(&url)?
                        .is_some_and(|r| r.changes > 0)
                {
                    detection.push(changed_at.elapsed().as_secs_f64() * 1000.0);
                }
                outcomes.push(outcome);
            } else {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            turns += 1;
            anyhow::ensure!(turns < 1000, "fixture failed to settle");
        }
    }
    sink.finish().await?;
    client.pipeline().shutdown().await;
    for url in urls {
        if let Some(resource) = frontier.revalidation_state(&url)? {
            captures.push(resource);
        } else {
            anyhow::ensure!(trap_test, "resource missing");
        }
    }
    let duration = start.elapsed().as_secs_f64() * 1000.0;
    let stats = frontier.metrics.snapshot();
    if trap_test {
        anyhow::ensure!(
            stats.commits == if config.mode == "protected" { 206 } else { 240 },
            "trap budget lost admitted URLs"
        );
    } else {
        anyhow::ensure!(
            stats.commits == 2 * config.urls as u64,
            "recrawl lost a durable result"
        );
    }
    let legitimate_covered = captures
        .iter()
        .filter(|r| Url::parse(&r.reference.url).is_ok_and(|u| u.path().starts_with("/page/")))
        .count();
    detection.sort_by(f64::total_cmp);
    Ok(
        json!({"legitimate_covered":legitimate_covered,"duration_ms":duration,"cpu_ms":cpu_ms()-cpu,"rss_after_bytes":rss(),"frontier":stats,"spool":writer.metrics.stats(),"body":client.body_reader().stats(),"extraction":client.extractor().stats(),"transport":transport.stats(),"outcomes":outcomes,"resources":captures,"detection_p95_ms":detection.get((detection.len()*95/100).min(detection.len().saturating_sub(1))).copied().unwrap_or(0.0)}),
    )
}
async fn tls(config: &Config) -> Result<Value> {
    let base: Url = config
        .http_base
        .as_deref()
        .context("missing TLS fixture")?
        .parse()?;
    anyhow::ensure!(
        base.scheme() == "https" && base.host_str() == Some("127.0.0.1"),
        "TLS fixture requires loopback"
    );
    let mut options = crate::config::CrawlerConfig::for_tests();
    options.network_policy = crate::config::CrawlNetworkPolicy::AllowPrivate;
    let transport = super::transport::SharedTransport::with_fixture_resolver(
        &options,
        Default::default(),
        Arc::new(Resolver),
        Arc::new(super::runtime::SystemClock),
        super::transport::FixturePolicy {
            allowed_hosts: ["resilience.test".into()].into(),
            certificate_pem: None,
        },
    )?;
    let url: Url = format!(
        "https://resilience.test:{}/",
        base.port().context("TLS port missing")?
    )
    .parse()?;
    let error = transport
        .send(url, None)
        .await
        .expect_err("an untrusted certificate must fail");
    let classified = matches!(&error, super::Error::Anyhow(cause) if super::retry::tls_validation_failure(cause));
    anyhow::ensure!(
        classified,
        "TLS validation did not retain an explicit certificate cause: {error:?}"
    );
    Ok(
        json!({"tls_validation_failed":true,"verification_disabled":false,"transport":transport.stats()}),
    )
}
pub async fn run(config: Config) -> Result<Value> {
    fs::create_dir_all(&config.directory)?;
    match config.action.as_str() {
        "planner" => planner(&config),
        "discovery" => discovery(&config),
        "diversity" => diversity(&config),
        "recrawl" | "traps" => recrawl(&config).await,
        "tls" => tls(&config).await,
        _ => anyhow::bail!("unknown resilience benchmark action"),
    }
}
