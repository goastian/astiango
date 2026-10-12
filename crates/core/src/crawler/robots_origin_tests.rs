use super::tests::{Reply, ScriptedHttp};
use super::*;
use crate::crawler::runtime::{Clock, IoFuture, Metrics, Runtime};

struct ControlledClock {
    start: Instant,
    elapsed: Mutex<Duration>,
}
impl Clock for ControlledClock {
    fn now(&self) -> Instant {
        self.start + *self.elapsed.lock().unwrap()
    }
    fn utc_now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
    }
    fn sleep(&self, duration: Duration) -> IoFuture<'_, ()> {
        Box::pin(async move {
            *self.elapsed.lock().unwrap() += duration;
        })
    }
}
fn manager(
    replies: Vec<Reply>,
    capacity: usize,
) -> (RobotsTxtManager, Arc<ScriptedHttp>, Arc<ControlledClock>) {
    let mut config = CrawlerConfig::for_tests();
    config.network_policy = CrawlNetworkPolicy::AllowPrivate;
    config.min_crawl_delay_ms = 0;
    config.start_politeness_factor = 0;
    config.robots_txt_cache_sec = 2;
    config.robots.cache_entries = capacity;
    config.robots.failure_ttl_seconds = 1;
    let clock = Arc::new(ControlledClock {
        start: Instant::now(),
        elapsed: Mutex::new(Duration::ZERO),
    });
    let http = Arc::new(ScriptedHttp {
        replies: Mutex::new(replies.into()),
        requests: Mutex::new(Vec::new()),
    });
    let runtime = Arc::new(Runtime {
        clock: clock.clone(),
        http: http.clone(),
        metrics: Arc::new(Metrics::default()),
    });
    (
        RobotsTxtManager::with_runtime(&config, runtime),
        http,
        clock,
    )
}
fn reply(status: u16, body: &'static str) -> Reply {
    Reply {
        status,
        body,
        location: None,
    }
}

#[tokio::test]
async fn hundred_misses_download_once_and_ttl_reloads_changed_rules() {
    let (manager, http, clock) = manager(
        vec![
            reply(200, "User-agent: *\nDisallow: /a\n"),
            reply(200, "User-agent: *\nDisallow: /b\n"),
        ],
        2,
    );
    let url = Url::parse("https://cache.test/a").unwrap();
    let mut tasks = tokio::task::JoinSet::new();
    for _ in 0..100 {
        let manager = manager.clone();
        let url = url.clone();
        tasks.spawn(async move { manager.access(&url).await });
    }
    while let Some(result) = tasks.join_next().await {
        assert_eq!(result.unwrap(), RobotAccess::Disallowed);
    }
    assert_eq!(http.requests.lock().unwrap().len(), 1);
    *clock.elapsed.lock().unwrap() += Duration::from_secs(2);
    assert_eq!(manager.access(&url).await, RobotAccess::Allowed);
    assert_eq!(http.requests.lock().unwrap().len(), 2);
    assert_eq!(manager.stats().downloads, 2);
}

#[tokio::test]
async fn scheme_and_port_rules_never_share_cache_and_cache_is_bounded() {
    let (manager, http, _) = manager(
        vec![
            reply(200, "User-agent: *\nDisallow: /a\n"),
            reply(200, "User-agent: *\nAllow: /\n"),
            reply(200, "User-agent: *\nDisallow: /a\n"),
        ],
        2,
    );
    for (url, access) in [
        ("http://origin.test:8000/a", RobotAccess::Disallowed),
        ("https://origin.test:8000/a", RobotAccess::Allowed),
        ("http://origin.test:8001/a", RobotAccess::Disallowed),
    ] {
        assert_eq!(manager.access(&Url::parse(url).unwrap()).await, access);
    }
    assert_eq!(http.requests.lock().unwrap().len(), 3);
    assert_eq!(manager.stats().entries, 2);
    assert_eq!(manager.stats().evictions, 1);
}

#[tokio::test]
async fn unavailable_and_failure_policies_are_explicit_and_temporary() {
    for (status, expected) in [
        (404, RobotAccess::Allowed),
        (403, RobotAccess::Allowed),
        (429, RobotAccess::Unreachable),
        (503, RobotAccess::Unreachable),
    ] {
        let (manager, http, clock) = manager(
            vec![reply(status, ""), reply(200, "User-agent: *\nAllow: /\n")],
            2,
        );
        let url = Url::parse("https://failure.test/a").unwrap();
        assert_eq!(manager.access(&url).await, expected);
        assert_eq!(manager.access(&url).await, expected);
        assert_eq!(http.requests.lock().unwrap().len(), 1);
        *clock.elapsed.lock().unwrap() += Duration::from_secs(2);
        assert_eq!(manager.access(&url).await, RobotAccess::Allowed);
        assert_eq!(http.requests.lock().unwrap().len(), 2);
    }
}

#[tokio::test]
async fn fair_robots_redirect_resumes_without_repeating_the_first_hop() {
    let mut config = CrawlerConfig::for_tests();
    config.min_crawl_delay_ms = 10;
    config.start_politeness_factor = 0;
    config.scheduler.mode = super::super::scheduler::Mode::FairTurns;
    let clock = Arc::new(ControlledClock {
        start: Instant::now(),
        elapsed: Mutex::new(Duration::ZERO),
    });
    let http = Arc::new(ScriptedHttp {
        replies: Mutex::new(
            vec![
                Reply {
                    status: 302,
                    location: Some("/policy"),
                    body: "",
                },
                reply(200, "User-agent: *\nDisallow: /blocked\n"),
            ]
            .into(),
        ),
        requests: Mutex::new(Vec::new()),
    });
    let manager = RobotsTxtManager::with_runtime(
        &config,
        Arc::new(Runtime {
            clock: clock.clone(),
            http: http.clone(),
            metrics: Arc::new(Metrics::default()),
        }),
    );
    let url = Url::parse("https://redirect.test/blocked").unwrap();
    assert_eq!(manager.access(&url).await, RobotAccess::Deferred(10));
    *clock.elapsed.lock().unwrap() += Duration::from_millis(10);
    assert_eq!(manager.access(&url).await, RobotAccess::Disallowed);
    assert_eq!(
        *http.requests.lock().unwrap(),
        [
            "https://redirect.test/robots.txt",
            "https://redirect.test/policy"
        ]
    );
}

#[tokio::test]
async fn cancelling_a_refresh_releases_singleflight_and_the_origin_permit() {
    use crate::crawler::runtime::Http;
    struct CancelHttp {
        calls: AtomicU64,
        started: tokio::sync::Notify,
    }
    impl Http for CancelHttp {
        fn send(
            &self,
            _: Url,
            _: Option<Duration>,
        ) -> IoFuture<'_, super::super::Result<reqwest::Response>> {
            Box::pin(async move {
                if self.calls.fetch_add(1, Ordering::Relaxed) == 0 {
                    self.started.notify_one();
                    std::future::pending::<()>().await;
                }
                Ok(http_02::Response::builder()
                    .status(200)
                    .body("User-agent: *\nAllow: /\n")
                    .unwrap()
                    .into())
            })
        }
    }
    let mut config = CrawlerConfig::for_tests();
    config.min_crawl_delay_ms = 0;
    config.start_politeness_factor = 0;
    let http = Arc::new(CancelHttp {
        calls: AtomicU64::new(0),
        started: tokio::sync::Notify::new(),
    });
    let clock = Arc::new(ControlledClock {
        start: Instant::now(),
        elapsed: Mutex::new(Duration::ZERO),
    });
    let manager = RobotsTxtManager::with_runtime(
        &config,
        Arc::new(Runtime {
            clock,
            http: http.clone(),
            metrics: Arc::new(Metrics::default()),
        }),
    );
    let url = Url::parse("https://cancel.test/page").unwrap();
    let task = {
        let manager = manager.clone();
        let url = url.clone();
        tokio::spawn(async move { manager.access(&url).await })
    };
    http.started.notified().await;
    task.abort();
    let _ = task.await;
    assert_eq!(manager.scheduler().stats().active_requests, 0);
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(1), manager.access(&url))
            .await
            .unwrap(),
        RobotAccess::Allowed
    );
    assert_eq!(http.calls.load(Ordering::Relaxed), 2);
}
