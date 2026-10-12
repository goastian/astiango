use super::*;
use crate::crawler::{
    observer::{Counters, FrontierState, Observer, Snapshot, Warc},
    runtime::{Clock, Http, IoFuture, Metrics, Runtime},
    Job,
};
use std::{collections::BTreeMap, sync::Mutex, time::Instant};

struct ManualClock {
    start: Instant,
    elapsed: Mutex<Duration>,
    sleeps: Mutex<Vec<Duration>>,
}
impl Default for ManualClock {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
            sleeps: Mutex::new(Vec::new()),
        }
    }
}
impl Clock for ManualClock {
    fn now(&self) -> Instant {
        self.start + *self.elapsed.lock().unwrap()
    }
    fn utc_now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
            + chrono::Duration::from_std(*self.elapsed.lock().unwrap()).unwrap()
    }
    fn sleep(&self, duration: Duration) -> IoFuture<'_, ()> {
        Box::pin(async move {
            self.sleeps.lock().unwrap().push(duration);
            *self.elapsed.lock().unwrap() += duration;
        })
    }
}

#[derive(Default)]
struct FixtureHttp {
    requests: Mutex<Vec<String>>,
    counts: Mutex<BTreeMap<String, usize>>,
}
impl Http for FixtureHttp {
    fn send(&self, url: Url, _: Option<Duration>) -> IoFuture<'_, Result<reqwest::Response>> {
        Box::pin(async move {
            self.requests.lock().unwrap().push(url.to_string());
            let count = {
                let mut counts = self.counts.lock().unwrap();
                let count = counts.entry(url.to_string()).or_default();
                *count += 1;
                *count
            };
            let path = url.path();
            if path == "/hang"
                || (path == "/robots.txt" && url.host_str() == Some("hang.example.com"))
            {
                return std::future::pending().await;
            }
            if path == "/hang-body" {
                let body = reqwest::Body::wrap_stream(futures::stream::pending::<
                    std::result::Result<Vec<u8>, std::io::Error>,
                >());
                return Ok(http_02::Response::builder()
                    .status(200)
                    .header("content-type", "text/html")
                    .body(body)
                    .unwrap()
                    .into());
            }
            if path == "/transport-error" {
                return Err(anyhow!("injected network failure").into());
            }
            let (status, location, retry_after, content_type, body) = match path {
                "/robots.txt" if url.host_str() == Some("down.example.com") => (503, None, None, None, ""),
                "/robots.txt" => (200, None, None, None, "User-agent: *\nDisallow: /blocked\n"),
                "/relative" => (302, Some("next".to_owned()), None, None, ""),
                "/cross-origin" => (307, Some("https://other.example.com/final".to_owned()), None, None, ""),
                "/cross-domain" => (302, Some("https://outside.example.net/final".to_owned()), None, None, ""),
                "/cycle-a" => (301, Some("/cycle-b".to_owned()), None, None, ""),
                "/cycle-b" => (308, Some("/cycle-a".to_owned()), None, None, ""),
                "/redirect-blocked" => (303, Some("/blocked".to_owned()), None, None, ""),
                "/no-location" => (301, None, None, None, ""),
                "/retry-once" if count == 1 => (429, None, Some("0"), None, ""),
                "/retry-forever" => (429, None, Some("0"), None, ""),
                "/retry-later" => (429, None, Some("86400"), None, ""),
                "/not-found" => (404, None, None, None, ""),
                "/unavailable" => (503, None, None, None, ""),
                "/invalid-mime" => (200, None, None, Some("application/json"), "{}"),
                value if value.starts_with("/chain/") => {
                    let next = value.trim_start_matches("/chain/").parse::<u16>().unwrap() + 1;
                    (302, Some(format!("/chain/{next}")), None, None, "")
                }
                _ => (200, None, None, Some("text/html"), "<!doctype html><html><head><title>fixture</title></head><body>saved</body></html>"),
            };
            let mut response = http_02::Response::builder().status(status);
            if let Some(location) = location {
                response = response.header("location", location);
            }
            if let Some(retry_after) = retry_after {
                response = response.header("retry-after", retry_after);
            }
            if let Some(content_type) = content_type {
                response = response.header("content-type", content_type);
            }
            Ok(response.body(body).unwrap().into())
        })
    }
}

#[derive(Default)]
struct MemorySink {
    body_reader: Mutex<Option<crate::crawler::body::BodyReader>>,
    data: Mutex<Vec<CrawlDatum>>,
    write_error: bool,
    checkpoint_error: bool,
    checkpoints: std::sync::atomic::AtomicUsize,
    cancel_after_write: Option<Arc<AtomicBool>>,
}
impl DatumSink for MemorySink {
    async fn write(&self, datum: CrawlDatum) -> Result<()> {
        if let Some(reader) = self.body_reader.lock().unwrap().as_ref() {
            assert!(
                reader.stats().active_bodies > 0,
                "sink must retain the body reservation"
            );
        }
        if self.write_error {
            return Err(anyhow!("injected disk failure").into());
        }
        self.data.lock().unwrap().push(datum);
        if let Some(flag) = &self.cancel_after_write {
            flag.store(true, Ordering::Release);
        }
        Ok(())
    }
    async fn checkpoint(&self) -> Result<()> {
        self.checkpoints.fetch_add(1, Ordering::Relaxed);
        if self.checkpoint_error {
            return Err(anyhow!("injected checkpoint failure").into());
        }
        Ok(())
    }
    async fn finish(&self) -> Result<()> {
        Ok(())
    }
}

fn config() -> CrawlerConfig {
    let mut config = CrawlerConfig::for_tests();
    config.min_crawl_delay_ms = 10;
    config.max_crawl_delay_ms = 100;
    config.start_politeness_factor = 0;
    config.min_politeness_factor = 0;
    config.max_url_slowdown_retry = 2;
    config.max_redirect_hops = 2;
    config
}

struct Run {
    outcome: JobOutcome,
    requests: Vec<String>,
    sleeps: Vec<Duration>,
    saved: Vec<(String, String)>,
    observation: Option<serde_json::Value>,
}

async fn run(
    paths: &[&str],
    observed: bool,
    sink: Arc<MemorySink>,
    cancellation: Option<Arc<AtomicBool>>,
) -> Run {
    run_config(paths, observed, sink, cancellation, config()).await
}

async fn run_config(
    paths: &[&str],
    observed: bool,
    sink: Arc<MemorySink>,
    cancellation: Option<Arc<AtomicBool>>,
    config: CrawlerConfig,
) -> Run {
    let http = Arc::new(FixtureHttp::default());
    let clock = Arc::new(ManualClock::default());
    let runtime = Arc::new(Runtime {
        http: http.clone(),
        clock: clock.clone(),
        metrics: Arc::new(Metrics::default()),
    });
    let client = RobotClient::with_runtime(&config, runtime).unwrap();
    let body_reader = client.body_reader().clone();
    *sink.body_reader.lock().unwrap() = Some(body_reader.clone());
    let base = Url::parse("https://one.example.com/").unwrap();
    let domain = Domain::from(&base);
    let urls = paths
        .iter()
        .map(|path| WeightedUrl {
            url: base.join(path).unwrap(),
            weight: 1.0,
        })
        .collect();
    let job = Job {
        domain: domain.clone(),
        urls,
        wandering_urls: 0,
    };
    let mut executor = JobExecutor::new(job.into(), Arc::new(config), sink.clone(), client);
    let directory = tempfile::tempdir().unwrap();
    let observation = directory.path().join("observer.json");
    let mut telemetry = None;
    if observed {
        let now = "2023-11-14T22:13:20Z".to_string();
        let observer = Arc::new(
            Observer::new(
                observation.clone(),
                Snapshot {
                    schema_version: 1,
                    run_id: "c02-test".into(),
                    pid: std::process::id(),
                    started_at: now.clone(),
                    updated_at: now,
                    finished_at: None,
                    phase: "running".into(),
                    user_agent: "fixture".into(),
                    workers: 1,
                    min_crawl_delay_ms: 10,
                    pages_per_domain: paths.len() as u64,
                    continuous: false,
                    frontier: FrontierState::default(),
                    counters: Counters::default(),
                    sites: Vec::new(),
                    recent_events: Vec::new(),
                    history: Vec::new(),
                    warc: Warc {
                        directory: directory.path().display().to_string(),
                        files: 0,
                        bytes: 0,
                        current_file: None,
                    },
                },
            )
            .unwrap(),
        );
        observer.observe_body(body_reader.clone());
        observer.register_site(domain.as_str().to_owned(), paths.len() as u64);
        executor = executor.with_observer(observer.clone());
        telemetry = Some(observer);
    }
    if let Some(flag) = cancellation {
        executor = executor.with_cancellation(flag);
    }
    let outcome = executor.run().await;
    assert_eq!(body_reader.stats().reserved_bytes, 0);
    assert_eq!(body_reader.stats().active_bodies, 0);
    if let Some(observer) = telemetry {
        observer.flush().unwrap();
    }
    let observation =
        observed.then(|| serde_json::from_slice(&std::fs::read(observation).unwrap()).unwrap());
    let requests = http.requests.lock().unwrap().clone();
    let sleeps = clock.sleeps.lock().unwrap().clone();
    let saved = sink
        .data
        .lock()
        .unwrap()
        .iter()
        .map(|datum| (datum.url.to_string(), datum.body.clone()))
        .collect();
    Run {
        outcome,
        requests,
        sleeps,
        saved,
        observation,
    }
}

#[tokio::test]
async fn telemetry_does_not_change_requests_decisions_delays_or_page_budget() {
    let paths = [
        "/relative",
        "/cross-origin",
        "/cross-domain",
        "/retry-once",
        "/blocked",
        "/not-found",
        "/unavailable",
        "/invalid-mime",
        "/cycle-a",
        "/chain/0",
        "/redirect-blocked",
        "/no-location",
        "/ok",
    ];
    let off = run(&paths, false, Arc::new(MemorySink::default()), None).await;
    let on = run(&paths, true, Arc::new(MemorySink::default()), None).await;
    assert_eq!(off.outcome, on.outcome);
    assert_eq!(off.requests, on.requests);
    assert_eq!(off.sleeps, on.sleeps);
    assert_eq!(off.saved, on.saved);
    assert_eq!(on.outcome.counters.useful_pages, 4);
    assert_eq!(on.outcome.status, JobStatus::Deferred);
    assert!(!on.outcome.is_success());
    assert!(!on
        .requests
        .iter()
        .any(|url| url.contains("outside.example.net") || url.ends_with("/blocked")));
    assert_eq!(
        on.outcome.counters.page_http_attempts as usize,
        on.requests
            .iter()
            .filter(|url| !url.ends_with("/robots.txt"))
            .count()
    );
    let counters = &on.observation.unwrap()["counters"];
    assert_eq!(
        counters["page_http_attempts"],
        on.outcome.counters.page_http_attempts
    );
    assert_eq!(counters["redirect_hops"], on.outcome.counters.redirect_hops);
    assert_eq!(counters["attempted"], paths.len() - 1);
}

#[tokio::test]
async fn redirects_without_mime_relative_and_same_domain_cross_origin_save_real_pages() {
    let result = run(
        &["/relative", "/cross-origin"],
        false,
        Arc::new(MemorySink::default()),
        None,
    )
    .await;
    assert!(result.outcome.is_success());
    assert_eq!(result.outcome.counters.useful_pages, 2);
    assert_eq!(result.outcome.counters.redirect_hops, 2);
    assert_eq!(
        result
            .saved
            .iter()
            .map(|entry| entry.0.as_str())
            .collect::<Vec<_>>(),
        [
            "https://one.example.com/next",
            "https://other.example.com/final"
        ]
    );
    assert!(result.saved.iter().all(|entry| entry.1.contains("saved")));
    let cross_robots = result
        .requests
        .iter()
        .position(|url| url == "https://other.example.com/robots.txt")
        .unwrap();
    let cross_page = result
        .requests
        .iter()
        .position(|url| url == "https://other.example.com/final")
        .unwrap();
    assert!(cross_robots < cross_page);
}

#[tokio::test]
async fn redirects_have_explicit_cycle_limit_and_boundary_outcomes() {
    let result = run(
        &[
            "/cycle-a",
            "/chain/0",
            "/cross-domain",
            "/redirect-blocked",
            "/no-location",
        ],
        false,
        Arc::new(MemorySink::default()),
        None,
    )
    .await;
    let reasons = result
        .outcome
        .urls
        .iter()
        .map(|entry| match &entry.outcome {
            FetchOutcome::PermanentFailure { reason } | FetchOutcome::Skipped { reason } => {
                reason.as_str()
            }
            other => panic!("unexpected {other:?}"),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        reasons,
        [
            "redirect_cycle",
            "redirect_limit",
            "outside_job_domain",
            "robots_disallowed",
            "invalid_redirect"
        ]
    );
    assert_eq!(result.outcome.counters.useful_pages, 0);
    assert!(!result.outcome.is_success());
}

#[tokio::test]
async fn retry_after_beyond_ceiling_defers_remaining_urls_without_discarding_them() {
    let result = run(
        &["/retry-later", "/ok"],
        false,
        Arc::new(MemorySink::default()),
        None,
    )
    .await;
    assert_eq!(result.outcome.status, JobStatus::Deferred);
    assert_eq!(
        result.outcome.urls[0].outcome,
        FetchOutcome::Deferred {
            reason: "http_429".into(),
            retry_after_ms: Some(86_400_000)
        }
    );
    assert_eq!(result.outcome.counters.useful_pages, 0);
    assert_eq!(
        result.outcome.urls[1].outcome,
        FetchOutcome::Deferred {
            reason: "origin_rate_limited".into(),
            retry_after_ms: Some(86_400_000)
        }
    );
    assert!(!result.requests.iter().any(|url| url.ends_with("/ok")));
    assert!(result
        .sleeps
        .iter()
        .all(|delay| *delay <= Duration::from_millis(100)));
}

#[tokio::test]
async fn transient_network_and_robots_failures_are_not_silent_success() {
    let result = run(
        &["/transport-error", "https://down.example.com/page"],
        false,
        Arc::new(MemorySink::default()),
        None,
    )
    .await;
    assert_eq!(result.outcome.status, JobStatus::Deferred);
    assert!(!result.outcome.is_success());
    assert!(result
        .outcome
        .urls
        .iter()
        .all(|entry| matches!(entry.outcome, FetchOutcome::Deferred { .. })));
    assert_eq!(
        result.outcome.urls[1].outcome,
        FetchOutcome::Deferred {
            reason: "robots_unavailable".into(),
            retry_after_ms: None
        }
    );
    assert!(!result
        .requests
        .iter()
        .any(|url| url == "https://down.example.com/page"));
}

#[tokio::test]
async fn storage_write_failure_stops_work_and_defers_unattempted_pages() {
    let sink = Arc::new(MemorySink {
        write_error: true,
        ..Default::default()
    });
    let result = run(&["/ok", "/remaining"], false, sink, None).await;
    assert_eq!(result.outcome.status, JobStatus::StorageFailed);
    assert_eq!(result.outcome.counters.useful_pages, 0);
    assert!(matches!(
        result.outcome.urls[0].outcome,
        FetchOutcome::StorageFailure { .. }
    ));
    assert!(matches!(
        result.outcome.urls[1].outcome,
        FetchOutcome::Deferred { .. }
    ));
    assert!(!result
        .requests
        .iter()
        .any(|url| url.ends_with("/remaining")));
}

#[tokio::test]
async fn buffered_sink_requires_checkpoint_before_durable_success() {
    let sink = Arc::new(MemorySink {
        checkpoint_error: true,
        ..Default::default()
    });
    let result = run(&["/ok", "/second"], false, sink.clone(), None).await;
    assert_eq!(sink.checkpoints.load(Ordering::Relaxed), 1);
    assert_eq!(result.outcome.status, JobStatus::StorageFailed);
    assert_eq!(result.outcome.counters.useful_pages, 0);
    assert!(result
        .outcome
        .urls
        .iter()
        .all(|entry| matches!(entry.outcome, FetchOutcome::StorageFailure { .. })));
    assert_eq!(result.saved.len(), 2);
}

#[tokio::test]
async fn cancellation_is_explicit_and_keeps_confirmed_pages() {
    let flag = Arc::new(AtomicBool::new(true));
    let initial = run(
        &["/ok", "/second"],
        false,
        Arc::new(MemorySink::default()),
        Some(flag),
    )
    .await;
    assert_eq!(initial.outcome.status, JobStatus::Cancelled);
    assert!(initial.requests.is_empty());
    assert!(initial
        .outcome
        .urls
        .iter()
        .all(|entry| entry.outcome == FetchOutcome::Cancelled));
    let flag = Arc::new(AtomicBool::new(false));
    let sink = Arc::new(MemorySink {
        cancel_after_write: Some(flag.clone()),
        ..Default::default()
    });
    let during = run(&["/ok", "/second"], true, sink.clone(), Some(flag)).await;
    assert_eq!(during.outcome.status, JobStatus::Cancelled);
    assert_eq!(during.outcome.counters.useful_pages, 1);
    assert_eq!(sink.checkpoints.load(Ordering::Relaxed), 1);
    assert!(matches!(
        during.outcome.urls[0].outcome,
        FetchOutcome::DurableSuccess { .. }
    ));
    assert_eq!(during.outcome.urls[1].outcome, FetchOutcome::Cancelled);
}

#[tokio::test]
async fn cancellation_interrupts_pending_page_http_without_aborting_checkpoint() {
    let flag = Arc::new(AtomicBool::new(false));
    let stop = flag.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        stop.store(true, Ordering::Release);
    });
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        run(
            &["/ok", "/hang", "/remaining"],
            false,
            Arc::new(MemorySink::default()),
            Some(flag),
        ),
    )
    .await
    .unwrap();
    assert_eq!(result.outcome.status, JobStatus::Cancelled);
    assert_eq!(result.outcome.counters.useful_pages, 1);
    assert!(result.outcome.urls[1..]
        .iter()
        .all(|entry| entry.outcome == FetchOutcome::Cancelled));
}

#[tokio::test]
async fn rate_limit_retries_are_bounded_independently_of_telemetry() {
    let off = run(
        &["/retry-forever", "/remaining"],
        false,
        Arc::new(MemorySink::default()),
        None,
    )
    .await;
    let on = run(
        &["/retry-forever", "/remaining"],
        true,
        Arc::new(MemorySink::default()),
        None,
    )
    .await;
    assert_eq!(off.outcome, on.outcome);
    assert_eq!(off.requests, on.requests);
    assert_eq!(off.sleeps, on.sleeps);
    assert_eq!(
        on.requests
            .iter()
            .filter(|url| url.ends_with("/retry-forever"))
            .count(),
        3
    );
    assert_eq!(on.outcome.status, JobStatus::Deferred);
    assert!(on
        .outcome
        .urls
        .iter()
        .all(|entry| matches!(entry.outcome, FetchOutcome::Deferred { .. })));
    assert!(!on.requests.iter().any(|url| url.ends_with("/remaining")));
}

#[tokio::test]
async fn explicit_private_policy_is_shared_with_executor_and_is_observer_independent() {
    let paths = ["https://one.example.com:8080/page"];
    let public = run(&paths, false, Arc::new(MemorySink::default()), None).await;
    assert!(public.requests.is_empty());
    assert!(matches!(
        public.outcome.urls[0].outcome,
        FetchOutcome::Skipped { .. }
    ));
    let mut private = config();
    private.network_policy = CrawlNetworkPolicy::AllowPrivate;
    let off = run_config(
        &paths,
        false,
        Arc::new(MemorySink::default()),
        None,
        private.clone(),
    )
    .await;
    let on = run_config(&paths, true, Arc::new(MemorySink::default()), None, private).await;
    assert_eq!(off.outcome, on.outcome);
    assert_eq!(off.requests, on.requests);
    assert_eq!(off.sleeps, on.sleeps);
    assert!(on.outcome.is_success());
    assert_eq!(on.outcome.counters.useful_pages, 1);
    assert!(on.requests.iter().any(|url| url == paths[0]));
}

#[tokio::test]
async fn cancellation_interrupts_robots_and_response_body_waits() {
    for path in ["https://hang.example.com/page", "/hang-body"] {
        let flag = Arc::new(AtomicBool::new(false));
        let stop = flag.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            stop.store(true, Ordering::Release);
        });
        let result = tokio::time::timeout(
            Duration::from_secs(1),
            run(
                &[path, "/remaining"],
                false,
                Arc::new(MemorySink::default()),
                Some(flag),
            ),
        )
        .await
        .unwrap();
        assert_eq!(result.outcome.status, JobStatus::Cancelled);
        assert_eq!(result.outcome.counters.useful_pages, 0);
        assert!(result
            .outcome
            .urls
            .iter()
            .all(|entry| entry.outcome == FetchOutcome::Cancelled));
    }
}

#[tokio::test]
async fn body_rejection_and_timeout_have_explicit_outcomes_with_or_without_observer() {
    let paths = ["/invalid-mime", "/hang-body", "/ok"];
    let mut config = config();
    config.body.idle_timeout_ms = 10;
    let off = run_config(
        &paths,
        false,
        Arc::new(MemorySink::default()),
        None,
        config.clone(),
    )
    .await;
    let on = run_config(&paths, true, Arc::new(MemorySink::default()), None, config).await;
    assert_eq!(off.outcome, on.outcome);
    assert_eq!(off.requests, on.requests);
    assert_eq!(off.sleeps, on.sleeps);
    assert_eq!(
        on.outcome.urls[0].outcome,
        FetchOutcome::Skipped {
            reason: "unsupported_mime".into()
        }
    );
    assert_eq!(
        on.outcome.urls[1].outcome,
        FetchOutcome::Deferred {
            reason: "idle_timeout".into(),
            retry_after_ms: None
        }
    );
    assert_eq!(on.outcome.counters.useful_pages, 1);
    let observed = on.observation.unwrap();
    assert_eq!(observed["body"]["rejected_mime"], 1);
    assert_eq!(observed["body"]["idle_timeouts"], 1);
    assert_eq!(observed["body"]["reserved_bytes"], 0);
}
