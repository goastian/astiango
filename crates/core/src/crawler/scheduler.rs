use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use url::Url;

use super::{identity::UrlIdentity, runtime::Clock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    LongJobs,
    FairTurns,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub mode: Mode,
    pub max_tracked_origins: usize,
    pub max_active_per_site: usize,
    pub max_active_per_ip: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mode: Mode::LongJobs,
            max_tracked_origins: 4096,
            max_active_per_site: 2,
            max_active_per_ip: 8,
        }
    }
}
impl Config {
    pub fn local() -> Self {
        Self {
            mode: Mode::FairTurns,
            ..Self::default()
        }
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=65536).contains(&self.max_tracked_origins)
                && (1..=128).contains(&self.max_active_per_site)
                && (1..=128).contains(&self.max_active_per_ip),
            "invalid scheduler limits"
        );
        Ok(())
    }
}

#[derive(Default, Clone, Debug, Serialize)]
pub struct Stats {
    pub mode: String,
    pub requests_admitted: u64,
    pub requests_deferred: u64,
    pub wait_ms: u64,
    pub active_requests: usize,
    pub active_peak: usize,
    pub tracked_origins: usize,
    pub origin_limit: usize,
    pub site_limit: usize,
    pub origin_overflows: u64,
}
struct Origin {
    next: Instant,
    active: bool,
    floor: Duration,
    retry: super::retry::OriginState,
}
#[derive(Default)]
struct State {
    origins: HashMap<String, Origin>,
    sites: HashMap<String, usize>,
    stats: Stats,
    request_window: u64,
    window_attempts: u32,
}
struct Inner {
    config: Config,
    clock: Arc<dyn Clock>,
    minimum: Duration,
    maximum: Duration,
    factor: f64,
    retry: super::retry::Config,
    state: Mutex<State>,
}
#[derive(Clone)]
pub struct Scheduler(Arc<Inner>);

impl Scheduler {
    pub fn new(config: &crate::config::CrawlerConfig, clock: Arc<dyn Clock>) -> Self {
        let factor = 2.0_f64.powi(config.start_politeness_factor.min(30) as i32);
        Self(Arc::new(Inner {
            config: config.scheduler.clone(),
            clock,
            minimum: Duration::from_millis(config.min_crawl_delay_ms),
            maximum: Duration::from_millis(config.max_crawl_delay_ms),
            factor,
            retry: config.retry.clone(),
            state: Mutex::new(State::default()),
        }))
    }
    pub fn mode(&self) -> Mode {
        self.0.config.mode
    }
    pub fn stats(&self) -> Stats {
        let state = self.0.state.lock().unwrap();
        Stats {
            mode: match self.mode() {
                Mode::LongJobs => "long_jobs",
                Mode::FairTurns => "fair_turns",
            }
            .into(),
            tracked_origins: state.origins.len(),
            origin_limit: self.0.config.max_tracked_origins,
            site_limit: self.0.config.max_active_per_site,
            ..state.stats.clone()
        }
    }
    pub fn set_crawl_delay(&self, url: &Url, delay: Duration) {
        let key = url.origin().ascii_serialization();
        let mut state = self.0.state.lock().unwrap();
        if let Some(origin) = state.origins.get_mut(&key) {
            origin.floor = delay;
            origin.next = origin.next.max(self.0.clock.now() + delay);
        }
    }
    pub fn try_acquire(&self, url: &Url) -> Result<Permit, Duration> {
        let identity = UrlIdentity::from_url(url).map_err(|_| Duration::from_secs(60))?;
        let now = self.0.clock.now();
        let mut state = self.0.state.lock().unwrap();
        if !state.origins.contains_key(&identity.origin)
            && state.origins.len() >= self.0.config.max_tracked_origins
        {
            state
                .origins
                .retain(|_, origin| origin.active || origin.next > now);
            if state.origins.len() >= self.0.config.max_tracked_origins {
                state.stats.origin_overflows += 1;
                state.stats.requests_deferred += 1;
                return Err(Duration::from_millis(100));
            }
        }
        let utc = self.0.clock.utc_now().timestamp_millis().max(0) as u64;
        if utc.saturating_sub(state.request_window) >= 60_000 {
            state.request_window = utc;
            state.window_attempts = 0;
        }
        if state.window_attempts >= self.0.retry.global_attempts_per_minute {
            return Err(Duration::from_millis(
                60_000u64
                    .saturating_sub(utc.saturating_sub(state.request_window))
                    .max(1),
            ));
        }
        let origin = state
            .origins
            .entry(identity.origin.clone())
            .or_insert(Origin {
                next: now,
                active: false,
                floor: Duration::ZERO,
                retry: Default::default(),
            });
        if utc.saturating_sub(origin.retry.window_start_ms) >= 60_000 {
            origin.retry.window_start_ms = utc;
            origin.retry.window_attempts = 0;
        }
        if origin.retry.window_attempts >= self.0.retry.origin_attempts_per_minute {
            return Err(Duration::from_millis(
                60_000u64
                    .saturating_sub(utc.saturating_sub(origin.retry.window_start_ms))
                    .max(1),
            ));
        }
        if origin.active || origin.next > now {
            let wait = origin
                .next
                .saturating_duration_since(now)
                .max(Duration::from_millis(1));
            state.stats.requests_deferred += 1;
            return Err(wait);
        }
        if state.sites.get(&identity.tenant).copied().unwrap_or(0)
            >= self.0.config.max_active_per_site
        {
            state.stats.requests_deferred += 1;
            return Err(Duration::from_millis(10));
        }
        let origin = state.origins.get_mut(&identity.origin).unwrap();
        origin.active = true;
        origin.retry.window_attempts += 1;
        state.window_attempts += 1;
        *state.sites.entry(identity.tenant.clone()).or_default() += 1;
        state.stats.requests_admitted += 1;
        state.stats.active_requests += 1;
        state.stats.active_peak = state.stats.active_peak.max(state.stats.active_requests);
        Ok(Permit {
            scheduler: self.clone(),
            origin: identity.origin,
            site: identity.tenant,
            started: now,
        })
    }
    pub async fn acquire(&self, url: &Url) -> super::Result<Permit> {
        loop {
            match self.try_acquire(url) {
                Ok(permit) => return Ok(permit),
                Err(wait) if self.mode() == Mode::FairTurns || wait > self.0.maximum => {
                    return Err(super::Error::CrawlDeferred {
                        retry_after_ms: wait.as_millis().max(1).min(u64::MAX as u128) as u64,
                    })
                }
                Err(wait) => {
                    self.0.state.lock().unwrap().stats.wait_ms += wait.as_millis() as u64;
                    self.0.clock.sleep(wait).await;
                }
            }
        }
    }
}

pub struct Permit {
    scheduler: Scheduler,
    origin: String,
    site: String,
    started: Instant,
}

impl Permit {
    pub(super) fn feedback(&self, result: &super::Result<reqwest::Response>) {
        let now = self.scheduler.0.clock.utc_now();
        let utc = now.timestamp_millis().max(0) as u64;
        let mut state = self.scheduler.0.state.lock().unwrap();
        let Some(origin) = state.origins.get_mut(&self.origin) else {
            return;
        };
        let failed = match result {
            Ok(response) => matches!(response.status().as_u16(), 408 | 425 | 429 | 500..=599),
            Err(_) => true,
        };
        if failed {
            let remote = result
                .as_ref()
                .ok()
                .and_then(|r| super::retry::retry_after(r.headers(), now))
                .unwrap_or(0);
            let delay = self.scheduler.0.retry.delay(
                origin.retry.consecutive_failures + 1,
                remote,
                &self.origin,
            );
            origin.retry.failure(&self.scheduler.0.retry, utc, delay);
            origin.next = origin.next.max(
                self.scheduler.0.clock.now()
                    + Duration::from_millis(origin.retry.not_before_ms.saturating_sub(utc)),
            );
        } else if result
            .as_ref()
            .is_ok_and(|r| r.status().is_success() || r.status().as_u16() == 304)
        {
            origin.retry.success(utc);
            origin.next = origin
                .next
                .max(self.scheduler.0.clock.now() + Duration::from_millis(origin.retry.penalty_ms));
        }
    }
}
pub fn guard_response(
    mut response: reqwest::Response,
    permit: Arc<Permit>,
) -> super::Result<reqwest::Response> {
    use reqwest::ResponseBuilderExt;
    let url = response.url().clone();
    let mut builder = http_02::Response::builder()
        .status(response.status())
        .version(response.version());
    *builder.headers_mut().unwrap() = response.headers().clone();
    *builder.extensions_mut().unwrap() = std::mem::take(response.extensions_mut());
    builder = builder.url(url);
    let stream = GuardedBody {
        body: Box::pin(response.bytes_stream()),
        permit: Some(permit),
    };
    Ok(builder
        .body(reqwest::Body::wrap_stream(stream))
        .map_err(|e| anyhow::anyhow!(e))?
        .into())
}
struct GuardedBody<S> {
    body: std::pin::Pin<Box<S>>,
    permit: Option<Arc<Permit>>,
}
impl<S: futures::Stream> futures::Stream for GuardedBody<S> {
    type Item = S::Item;
    fn poll_next(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        let result = self.body.as_mut().poll_next(cx);
        if matches!(result, std::task::Poll::Ready(None)) {
            self.permit.take();
        }
        result
    }
}
impl Drop for Permit {
    fn drop(&mut self) {
        let inner = &self.scheduler.0;
        let now = inner.clock.now();
        let mut state = inner.state.lock().unwrap();
        if let Some(origin) = state.origins.get_mut(&self.origin) {
            origin.active = false;
            let adaptive = now
                .saturating_duration_since(self.started)
                .max(inner.minimum)
                .mul_f64(inner.factor)
                .min(inner.maximum);
            origin.next = origin
                .next
                .max(now + adaptive.max(origin.floor).max(inner.minimum));
        }
        if let Some(count) = state.sites.get_mut(&self.site) {
            *count -= 1;
            if *count == 0 {
                state.sites.remove(&self.site);
            }
        }
        state.stats.active_requests -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::super::runtime::{IoFuture, SystemClock};
    use super::*;
    struct ClockAt {
        start: Instant,
        elapsed: Mutex<Duration>,
    }
    impl Clock for ClockAt {
        fn now(&self) -> Instant {
            self.start + *self.elapsed.lock().unwrap()
        }
        fn utc_now(&self) -> chrono::DateTime<chrono::Utc> {
            chrono::Utc::now()
        }
        fn sleep(&self, duration: Duration) -> IoFuture<'_, ()> {
            Box::pin(async move {
                *self.elapsed.lock().unwrap() += duration;
            })
        }
    }
    #[test]
    fn leases_cover_body_cancellation_and_minimum_delay() {
        let mut config = crate::config::CrawlerConfig::for_tests();
        config.min_crawl_delay_ms = 10;
        config.start_politeness_factor = 0;
        let clock = Arc::new(ClockAt {
            start: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
        });
        let scheduler = Scheduler::new(&config, clock.clone());
        let url = Url::parse("https://alice.github.io/a").unwrap();
        let lease = scheduler.try_acquire(&url).ok().unwrap();
        assert!(scheduler.try_acquire(&url).is_err());
        drop(lease);
        assert_eq!(
            scheduler.try_acquire(&url).err(),
            Some(Duration::from_millis(10))
        );
        *clock.elapsed.lock().unwrap() += Duration::from_millis(10);
        assert!(scheduler.try_acquire(&url).is_ok());
        assert_eq!(scheduler.stats().active_requests, 0);
    }
    #[test]
    fn tenants_are_separate_and_crawl_delay_is_never_capped_away() {
        let mut config = crate::config::CrawlerConfig::for_tests();
        config.min_crawl_delay_ms = 0;
        config.max_crawl_delay_ms = 10;
        config.scheduler.max_active_per_site = 1;
        let scheduler = Scheduler::new(&config, Arc::new(SystemClock));
        let a = Url::parse("https://alice.github.io/a").unwrap();
        let b = Url::parse("https://bob.github.io/a").unwrap();
        let permit = scheduler.try_acquire(&a).ok().unwrap();
        let other = scheduler.try_acquire(&b).ok().unwrap();
        scheduler.set_crawl_delay(&a, Duration::from_secs(2));
        drop(permit);
        drop(other);
        assert!(scheduler.try_acquire(&a).err().unwrap() > Duration::from_secs(1));
    }
    #[tokio::test]
    async fn converting_to_a_body_stream_keeps_origin_active_until_eof_or_cancel() {
        use futures::StreamExt;
        let mut config = crate::config::CrawlerConfig::for_tests();
        config.min_crawl_delay_ms = 0;
        config.start_politeness_factor = 0;
        let scheduler = Scheduler::new(
            &config,
            Arc::new(ClockAt {
                start: Instant::now(),
                elapsed: Mutex::new(Duration::ZERO),
            }),
        );
        let url = Url::parse("https://origin.test/a").unwrap();
        let permit = Arc::new(scheduler.try_acquire(&url).ok().unwrap());
        let body = reqwest::Body::wrap_stream(futures::stream::pending::<
            Result<Vec<u8>, std::io::Error>,
        >());
        let response = guard_response(http_02::Response::new(body).into(), permit).unwrap();
        let stream = response.bytes_stream();
        assert_eq!(scheduler.stats().active_requests, 1);
        assert!(scheduler.try_acquire(&url).is_err());
        drop(stream);
        assert_eq!(scheduler.stats().active_requests, 0);
        let permit = Arc::new(scheduler.try_acquire(&url).ok().unwrap());
        let response = guard_response(http_02::Response::new("body").into(), permit).unwrap();
        let mut stream = response.bytes_stream();
        while stream.next().await.is_some() {}
        assert_eq!(scheduler.stats().active_requests, 0);
    }
    #[test]
    fn site_and_origin_capacity_are_bounded_and_expired_crawl_delay_can_decrease() {
        let mut config = crate::config::CrawlerConfig::for_tests();
        config.min_crawl_delay_ms = 0;
        config.start_politeness_factor = 0;
        config.scheduler.max_active_per_site = 1;
        config.scheduler.max_tracked_origins = 2;
        let clock = Arc::new(ClockAt {
            start: Instant::now(),
            elapsed: Mutex::new(Duration::ZERO),
        });
        let scheduler = Scheduler::new(&config, clock.clone());
        let a = Url::parse("https://a.example.com/a").unwrap();
        let b = Url::parse("https://b.example.com/a").unwrap();
        let permit = scheduler.try_acquire(&a).ok().unwrap();
        assert!(scheduler.try_acquire(&b).is_err());
        scheduler.set_crawl_delay(&a, Duration::from_secs(2));
        drop(permit);
        scheduler.set_crawl_delay(&a, Duration::ZERO);
        assert!(scheduler.try_acquire(&a).is_err());
        *clock.elapsed.lock().unwrap() += Duration::from_secs(2);
        drop(scheduler.try_acquire(&a).ok().unwrap());
        assert!(scheduler.try_acquire(&a).is_ok());
        assert!(scheduler.stats().tracked_origins <= 2);
    }
}
