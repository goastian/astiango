//! Injectable crawler I/O. Production constructors keep their existing network policy.
//! Injection is exposed only to tests and the explicitly enabled local benchmark.
use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use url::Url;

use super::Result;

pub type IoFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn utc_now(&self) -> chrono::DateTime<chrono::Utc>;
    fn sleep(&self, duration: Duration) -> IoFuture<'_, ()>;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn utc_now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }
    fn sleep(&self, duration: Duration) -> IoFuture<'_, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

pub trait Dns: Send + Sync {
    fn resolve<'a>(&'a self, host: &'a str) -> IoFuture<'a, Result<std::net::SocketAddr>>;
}

pub trait Http: Send + Sync {
    fn send(&self, url: Url, timeout: Option<Duration>) -> IoFuture<'_, Result<reqwest::Response>>;
}

#[derive(Default, serde::Serialize, Clone)]
pub struct Measurements {
    pub stages_ms: BTreeMap<String, f64>,
    pub client_requests: u64,
    pub requests_by_path: BTreeMap<String, u64>,
    pub requests_by_origin: BTreeMap<String, u64>,
    pub request_failures: u64,
    pub blocked_outbound: u64,
    pub errors: BTreeMap<String, u64>,
}

#[derive(Default)]
pub struct Metrics(Mutex<Measurements>);
impl Metrics {
    pub fn reset(&self) {
        *self.0.lock().unwrap() = Measurements::default();
    }
    pub fn snapshot(&self) -> Measurements {
        self.0.lock().unwrap().clone()
    }
    pub fn stage(&self, name: &str, elapsed: Duration) {
        *self
            .0
            .lock()
            .unwrap()
            .stages_ms
            .entry(name.into())
            .or_default() += elapsed.as_secs_f64() * 1000.0;
    }
    pub fn request(&self, url: &Url) {
        let mut m = self.0.lock().unwrap();
        m.client_requests += 1;
        *m.requests_by_path.entry(url.path().into()).or_default() += 1;
        *m.requests_by_origin
            .entry(url.host_str().unwrap_or_default().into())
            .or_default() += 1;
    }
    pub fn failure(&self) {
        self.0.lock().unwrap().request_failures += 1;
    }
    pub fn blocked(&self) {
        self.0.lock().unwrap().blocked_outbound += 1;
    }
    pub fn error(&self, error: &super::Error) {
        let label = match error {
            super::Error::FetchFailed { status_code, .. } => {
                format!("http_{}", status_code.as_u16())
            }
            super::Error::ContentTooLarge => "content_too_large".into(),
            super::Error::DisallowedPath => "robots_disallowed".into(),
            super::Error::InvalidHtml => "invalid_html".into(),
            _ => "other".into(),
        };
        *self.0.lock().unwrap().errors.entry(label).or_default() += 1;
    }
    pub fn timer(self: &Arc<Self>, stage: &'static str) -> StageTimer {
        StageTimer {
            metrics: Some(self.clone()),
            stage,
            start: Some(Instant::now()),
        }
    }
}

pub struct StageTimer {
    metrics: Option<Arc<Metrics>>,
    stage: &'static str,
    start: Option<Instant>,
}
impl StageTimer {
    pub(crate) fn optional(metrics: Option<&Arc<Metrics>>, stage: &'static str) -> Self {
        Self {
            metrics: metrics.cloned(),
            stage,
            start: metrics.map(|_| Instant::now()),
        }
    }
}
impl Drop for StageTimer {
    fn drop(&mut self) {
        if let (Some(metrics), Some(start)) = (&self.metrics, self.start) {
            metrics.stage(self.stage, start.elapsed());
        }
    }
}

pub struct Runtime {
    pub http: Arc<dyn Http>,
    pub clock: Arc<dyn Clock>,
    pub metrics: Arc<Metrics>,
}
impl Runtime {
    pub(crate) async fn sleep(&self, duration: Duration) {
        let _timer = self.metrics.timer("politeness");
        self.clock.sleep(duration).await;
    }
    pub(crate) async fn body(&self, response: reqwest::Response) -> Result<String> {
        let _timer = self.metrics.timer("http_body");
        super::encoded_body(response).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::CrawlerConfig,
        crawler::{
            robot_client::RobotClient, CrawlDatum, DatumSink, Domain, Job, JobExecutor, WeightedUrl,
        },
    };
    use std::collections::VecDeque;

    struct ManualClock {
        start: Instant,
        elapsed: Mutex<Duration>,
    }
    impl ManualClock {
        fn new() -> Self {
            Self {
                start: Instant::now(),
                elapsed: Mutex::new(Duration::ZERO),
            }
        }
        fn advance(&self, duration: Duration) {
            *self.elapsed.lock().unwrap() += duration;
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
                self.advance(duration);
            })
        }
    }
    #[derive(Default)]
    struct FixtureHttp {
        requests: Mutex<Vec<String>>,
    }
    impl Http for FixtureHttp {
        fn send(
            &self,
            url: Url,
            _timeout: Option<Duration>,
        ) -> IoFuture<'_, Result<reqwest::Response>> {
            Box::pin(async move {
                self.requests.lock().unwrap().push(url.path().into());
                let body = if url.path() == "/robots.txt" {
                    "User-agent: *\nDisallow: /blocked\n"
                } else {
                    "<html><head><title>Recorded fixture</title></head><body>page</body></html>"
                };
                Ok(http_02::Response::builder()
                    .status(200)
                    .header("content-type", "text/html")
                    .body(body)
                    .unwrap()
                    .into())
            })
        }
    }
    #[derive(Default)]
    struct MemorySink(Mutex<Vec<CrawlDatum>>);
    impl DatumSink for MemorySink {
        async fn write(&self, datum: CrawlDatum) -> Result<()> {
            self.0.lock().unwrap().push(datum);
            Ok(())
        }
        async fn finish(&self) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn injected_http_and_clock_execute_real_job_without_network_or_wall_sleep() {
        let mut config = CrawlerConfig::for_tests();
        config.min_crawl_delay_ms = 10_000;
        config.max_crawl_delay_ms = 10_000;
        config.start_politeness_factor = 0;
        config.min_politeness_factor = 0;
        let http = Arc::new(FixtureHttp::default());
        let clock = Arc::new(ManualClock::new());
        let runtime = Arc::new(Runtime {
            http: http.clone(),
            clock: clock.clone(),
            metrics: Arc::new(Metrics::default()),
        });
        let client = RobotClient::with_runtime(&config, runtime.clone()).unwrap();
        let sink = Arc::new(MemorySink::default());
        let base = Url::parse("http://fixture.test/").unwrap();
        let job = Job {
            domain: Domain::from(&base),
            urls: VecDeque::from([
                WeightedUrl {
                    url: base.join("blocked").unwrap(),
                    weight: 1.0,
                },
                WeightedUrl {
                    url: base.join("allowed").unwrap(),
                    weight: 1.0,
                },
            ]),
            wandering_urls: 0,
        };
        JobExecutor::new(job.into(), Arc::new(config), sink.clone(), client)
            .run()
            .await;
        assert_eq!(*http.requests.lock().unwrap(), ["/robots.txt", "/allowed"]);
        assert_eq!(sink.0.lock().unwrap().len(), 1);
        assert_eq!(*clock.elapsed.lock().unwrap(), Duration::from_secs(20));
        assert!(runtime.metrics.snapshot().stages_ms.contains_key("parse"));
    }

    #[tokio::test]
    async fn injected_clock_expires_robots_cache_deterministically() {
        let mut config = CrawlerConfig::for_tests();
        config.robots_txt_cache_sec = 60;
        config.min_crawl_delay_ms = 0;
        let http = Arc::new(FixtureHttp::default());
        let clock = Arc::new(ManualClock::new());
        let client = RobotClient::with_runtime(
            &config,
            Arc::new(Runtime {
                http: http.clone(),
                clock: clock.clone(),
                metrics: Arc::new(Metrics::default()),
            }),
        )
        .unwrap();
        let url = Url::parse("http://fixture.test/allowed").unwrap();
        assert!(client.robots_txt_manager().is_allowed(&url).await);
        clock.advance(Duration::from_secs(59));
        assert!(client.robots_txt_manager().is_allowed(&url).await);
        assert_eq!(http.requests.lock().unwrap().len(), 1);
        clock.advance(Duration::from_secs(1));
        assert!(client.robots_txt_manager().is_allowed(&url).await);
        assert_eq!(http.requests.lock().unwrap().len(), 2);
    }
}
