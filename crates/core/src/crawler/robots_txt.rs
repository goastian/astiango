// Stract is an open source web search engine.
// Copyright (C) 2024 Stract ApS
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use super::{
    body::{AdmittedBody, BodyKind, BodyReader},
    scheduler::Scheduler,
    transport::SharedTransport,
};
use crate::config::{CrawlNetworkPolicy, CrawlerConfig};
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use url::Url;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub cache_entries: usize,
    pub failure_ttl_seconds: u64,
}
impl Config {
    pub fn validate(&self, body: &super::body::BodyConfig) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=65536).contains(&self.cache_entries)
                && (1..=86400).contains(&self.failure_ttl_seconds),
            "invalid robots cache limits"
        );
        anyhow::ensure!(
            body.robots.wire_limit_bytes >= 500 * 1024
                && body.robots.decoded_limit_bytes >= 500 * 1024,
            "robots body limits must admit at least 500 KiB"
        );
        Ok(())
    }
}
impl Default for Config {
    fn default() -> Self {
        Self {
            cache_entries: 1024,
            failure_ttl_seconds: 60,
        }
    }
}

#[derive(Default, Clone, Debug, serde::Serialize)]
pub struct Stats {
    pub hits: u64,
    pub misses: u64,
    pub single_flight_waits: u64,
    pub downloads: u64,
    pub evictions: u64,
    pub capacity_deferred: u64,
    pub unreachable: u64,
    pub unavailable: u64,
    pub disallowed: u64,
    pub entries: usize,
    pub capacity: usize,
    pub ttl_seconds: u64,
    pub failure_ttl_seconds: u64,
}
#[derive(Default)]
struct Counters {
    hits: AtomicU64,
    misses: AtomicU64,
    waits: AtomicU64,
    downloads: AtomicU64,
    evictions: AtomicU64,
    capacity_deferred: AtomicU64,
    unreachable: AtomicU64,
    unavailable: AtomicU64,
    disallowed: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotAccess {
    Allowed,
    Disallowed,
    Unreachable,
    Deferred(u64),
}
#[derive(Debug)]
enum Lookup<T> {
    Found(T),
    Unavailable,
    Unreachable,
    Deferred(u64),
}
struct CheckedRobotsTxt {
    robots: Lookup<robotstxt::Robots>,
    last_check: Instant,
    ttl: Duration,
}
impl CheckedRobotsTxt {
    #[cfg(test)]
    fn is_expired(&self, expiration: &Duration) -> bool {
        self.is_expired_at(expiration, Instant::now())
    }
    fn is_expired_at(&self, expiration: &Duration, now: Instant) -> bool {
        now.saturating_duration_since(self.last_check) >= *expiration
    }
}
struct CacheEntry {
    value: Option<Arc<CheckedRobotsTxt>>,
    refreshing: bool,
    touched: Instant,
    continuation: Option<(Url, usize)>,
    consumed: bool,
}
struct InnerRobotsTxtManager {
    cache: Mutex<HashMap<String, CacheEntry>>,
    changed: tokio::sync::Notify,
    counters: Counters,
    transport: SharedTransport,
    body_reader: BodyReader,
    pipeline: super::pipeline::Pipeline,
    cache_expiration: Duration,
    user_agent: String,
    config: Arc<CrawlerConfig>,
    scheduler: Scheduler,
    runtime: Option<Arc<super::runtime::Runtime>>,
}
struct Refresh<'a> {
    manager: &'a InnerRobotsTxtManager,
    origin: String,
}
impl Drop for Refresh<'_> {
    fn drop(&mut self) {
        if let Some(entry) = self.manager.cache.lock().unwrap().get_mut(&self.origin) {
            entry.refreshing = false;
        }
        self.manager.changed.notify_waiters();
    }
}
impl InnerRobotsTxtManager {
    fn new(
        config: &CrawlerConfig,
        transport: SharedTransport,
        body_reader: BodyReader,
        pipeline: super::pipeline::Pipeline,
    ) -> Self {
        Self {
            cache: Mutex::new(HashMap::new()),
            changed: tokio::sync::Notify::new(),
            counters: Counters::default(),
            transport,
            body_reader,
            pipeline,
            cache_expiration: Duration::from_secs(config.robots_txt_cache_sec.min(86400)),
            user_agent: config.user_agent.token.clone(),
            config: Arc::new(config.clone()),
            scheduler: Scheduler::new(config, Arc::new(super::runtime::SystemClock)),
            runtime: None,
        }
    }
    async fn access(&self, url: &Url) -> RobotAccess {
        if super::robot_client::validate_network_url(self.config.network_policy, url).is_err() {
            return RobotAccess::Disallowed;
        }
        match &self.get(url).await.robots {
            Lookup::Found(rules) if rules.is_allowed(url) => RobotAccess::Allowed,
            Lookup::Found(_) => {
                self.counters.disallowed.fetch_add(1, Ordering::Relaxed);
                self.consumed(url);
                RobotAccess::Disallowed
            }
            Lookup::Unavailable => RobotAccess::Allowed,
            Lookup::Unreachable => { self.consumed(url); RobotAccess::Unreachable },
            Lookup::Deferred(ms) => RobotAccess::Deferred(*ms),
        }
    }
    fn consumed(&self, url: &Url) {
        if let Some(entry) = self.cache.lock().unwrap().get_mut(&url.origin().ascii_serialization()) {
            entry.consumed = true;
        }
        self.changed.notify_waiters();
    }
    async fn crawl_delay(&self, url: &Url) -> Option<Duration> {
        match &self.get(url).await.robots {
            Lookup::Found(rules) => rules.crawl_delay(),
            _ => None,
        }
    }
    async fn sitemaps(&self, url: &Url) -> Vec<Url> {
        match &self.get(url).await.robots {
            Lookup::Found(rules) => rules
                .sitemaps()
                .iter()
                .filter_map(|s| Url::parse(s).ok())
                .collect(),
            _ => vec![],
        }
    }
    fn now(&self) -> Instant {
        self.runtime
            .as_ref()
            .map_or_else(Instant::now, |runtime| runtime.clock.now())
    }
    fn utc_now(&self) -> chrono::DateTime<chrono::Utc> {
        self.runtime
            .as_ref()
            .map_or_else(chrono::Utc::now, |runtime| runtime.clock.utc_now())
    }
    async fn body(&self, response: reqwest::Response) -> super::Result<AdmittedBody> {
        let metrics = self.runtime.as_ref().map(|runtime| &runtime.metrics);
        let _timer = super::runtime::StageTimer::optional(metrics, "http_body");
        self.body_reader
            .read(response, BodyKind::Robots)
            .await
            .map_err(super::Error::from)
    }
    async fn request(&self, url: Url) -> super::Result<super::robot_client::RequestBuilder> {
        super::robot_client::validate_network_url(self.config.network_policy, &url)?;
        let request = match &self.runtime {
            Some(runtime) => super::robot_client::RequestBuilder::injected(
                runtime.clone(),
                url.clone(),
                &self.config,
            ),
            None => {
                super::robot_client::RequestBuilder::managed(self.transport.clone(), url.clone())
            }
        };
        request
            .with_scheduler(self.scheduler.clone(), url)
            .prepare()
            .await
    }
    async fn fetch(&self, initial: &Url) -> CheckedRobotsTxt {
        let mut url = initial.clone();
        url.set_path("/robots.txt");
        url.set_query(None);
        url.set_fragment(None);
        let origin = initial.origin().ascii_serialization();
        let continuation = self
            .cache
            .lock()
            .unwrap()
            .get_mut(&origin)
            .and_then(|entry| entry.continuation.take());
        let mut first_hop = 0;
        if let Some((next, hop)) = continuation {
            url = next;
            first_hop = hop;
        }
        let mut ttl = self.cache_expiration;
        let mut lookup = Lookup::Unreachable;
        for hop in first_hop..=5 {
            let request = match self.request(url.clone()).await {
                Ok(request) => request,
                Err(super::Error::CrawlDeferred { retry_after_ms }) => {
                    if let Some(entry) = self.cache.lock().unwrap().get_mut(&origin) {
                        entry.continuation = Some((url.clone(), hop));
                    }
                    lookup = Lookup::Deferred(retry_after_ms);
                    break;
                }
                Err(_) => break,
            };
            self.counters.downloads.fetch_add(1, Ordering::Relaxed);
            let response = match request.send().await {
                Ok(response) => response,
                Err(_) => break,
            };
            let status = response.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                let next = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| url.join(v).ok());
                drop(response);
                if let Some(next) = next {
                    url = next;
                    continue;
                }
                break;
            }
            if status == 429 {
                let retry_after = response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| {
                        v.parse::<u64>().ok().map(Duration::from_secs).or_else(|| {
                            chrono::DateTime::parse_from_rfc2822(v).ok().map(|date| {
                                Duration::from_secs(
                                    (date.with_timezone(&chrono::Utc) - self.utc_now())
                                        .num_seconds()
                                        .max(0) as u64,
                                )
                            })
                        })
                    });
                ttl = self.failure_ttl().max(
                    retry_after
                        .unwrap_or_default()
                        .min(Duration::from_secs(31_536_000)),
                );
                break;
            }
            if (400..500).contains(&status) {
                lookup = Lookup::Unavailable;
                self.counters.unavailable.fetch_add(1, Ordering::Relaxed);
                break;
            }
            if !(200..300).contains(&status) {
                break;
            }
            let body = match self.body(response).await {
                Ok(body) => body,
                Err(_) => break,
            };
            let agent = self.user_agent.clone();
            self.pipeline.start_monitor();
            lookup = self
                .pipeline
                .parse
                .run(body.len(), move || {
                    robotstxt::Robots::parse_rfc(&agent, &body)
                        .map(Lookup::Found)
                        .unwrap_or(Lookup::Unreachable)
                })
                .await
                .unwrap_or(Lookup::Unreachable);
            if let Lookup::Found(rules) = &lookup {
                self.scheduler
                    .set_crawl_delay(initial, rules.crawl_delay().unwrap_or_default());
            }
            break;
        }
        if matches!(lookup, Lookup::Unreachable) {
            self.counters.unreachable.fetch_add(1, Ordering::Relaxed);
            ttl = if ttl == self.cache_expiration {
                self.failure_ttl()
            } else {
                ttl
            };
        }
        if matches!(lookup, Lookup::Unavailable) {
            self.scheduler.set_crawl_delay(initial, Duration::ZERO);
        }
        CheckedRobotsTxt {
            robots: lookup,
            last_check: self.now(),
            ttl,
        }
    }
    fn failure_ttl(&self) -> Duration {
        Duration::from_secs(self.config.robots.failure_ttl_seconds.max(1).min(86400))
    }
    async fn get(&self, url: &Url) -> Arc<CheckedRobotsTxt> {
        let origin = url.origin().ascii_serialization();
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let fetch = {
                let mut cache = self.cache.lock().unwrap();
                let now = self.now();
                if let Some(entry) = cache.get_mut(&origin) {
                    entry.touched = now;
                    if let Some(value) = &entry.value {
                        if !value.is_expired_at(&value.ttl, now) {
                            self.counters.hits.fetch_add(1, Ordering::Relaxed);
                            return value.clone();
                        }
                    }
                    if entry.refreshing {
                        false
                    } else {
                        entry.refreshing = true;
                        true
                    }
                } else {
                    if cache.len() >= self.config.robots.cache_entries.max(1) {
                        let oldest = cache
                            .iter()
                            .filter(|(_, entry)| !entry.refreshing && (self.scheduler.mode() != super::scheduler::Mode::FairTurns || entry.consumed
                                || entry.value.as_ref().is_some_and(|value| value.is_expired_at(&value.ttl, now))))
                            .min_by_key(|(_, entry)| entry.touched)
                            .map(|(key, _)| key.clone());
                        if let Some(oldest) = oldest {
                            cache.remove(&oldest);
                            self.counters.evictions.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    if cache.len() < self.config.robots.cache_entries.max(1) {
                        cache.insert(
                            origin.clone(),
                            CacheEntry {
                                value: None,
                                refreshing: true,
                                touched: now,
                                continuation: None,
                                consumed: false,
                            },
                        );
                        true
                    } else {
                        // Preserve rules fetched for a page that is still waiting
                        // for courtesy. Returning a deferred outcome releases the
                        // worker so one of those cached origins can make progress.
                        self.counters.capacity_deferred.fetch_add(1, Ordering::Relaxed);
                        return Arc::new(CheckedRobotsTxt {
                            robots: Lookup::Deferred(self.config.min_crawl_delay_ms.max(1000)),
                            last_check: now,
                            ttl: Duration::ZERO,
                        });
                    }
                }
            };
            if !fetch {
                self.counters.waits.fetch_add(1, Ordering::Relaxed);
                notified.await;
                continue;
            }
            let refresh = Refresh {
                manager: self,
                origin: origin.clone(),
            };
            self.counters.misses.fetch_add(1, Ordering::Relaxed);
            let value = Arc::new(self.fetch(url).await);
            if !matches!(value.robots, Lookup::Deferred(_)) {
                if let Some(entry) = self.cache.lock().unwrap().get_mut(&origin) {
                    entry.value = Some(value.clone());
                    entry.consumed = matches!(value.robots, Lookup::Unreachable);
                }
            }
            drop(refresh);
            return value;
        }
    }
    #[cfg(test)]
    fn insert(&self, site: String, rules: robotstxt::Robots) {
        let origins = if site.contains("://") {
            vec![Url::parse(&site).unwrap().origin().ascii_serialization()]
        } else {
            vec![format!("https://{site}"), format!("http://{site}")]
        };
        let value = Arc::new(CheckedRobotsTxt {
            robots: Lookup::Found(rules),
            last_check: self.now(),
            ttl: self.cache_expiration,
        });
        for origin in origins {
            self.cache.lock().unwrap().insert(
                origin,
                CacheEntry {
                    value: Some(value.clone()),
                    refreshing: false,
                    touched: self.now(),
                    continuation: None,
                    consumed: false,
                },
            );
        }
    }
}
pub struct RobotsTxtManager {
    inner: Arc<InnerRobotsTxtManager>,
}

impl Clone for RobotsTxtManager {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl RobotsTxtManager {
    pub fn new(config: &CrawlerConfig) -> Self {
        let transport = SharedTransport::new(config, config.transport.clone())
            .expect("invalid crawler transport configuration");
        Self::with_transport(config, transport)
    }

    pub(crate) fn with_transport(config: &CrawlerConfig, transport: SharedTransport) -> Self {
        let body_reader = BodyReader::new(config.body.clone()).expect("invalid body configuration");
        Self::with_readers(config, transport, body_reader)
    }

    pub(crate) fn with_readers(
        config: &CrawlerConfig,
        transport: SharedTransport,
        body_reader: BodyReader,
    ) -> Self {
        Self::with_pipeline_readers(
            config,
            transport,
            body_reader,
            super::pipeline::Pipeline::new(config.pipeline.clone())
                .expect("invalid pipeline configuration"),
        )
    }

    pub(crate) fn with_pipeline_readers(
        config: &CrawlerConfig,
        transport: SharedTransport,
        body_reader: BodyReader,
        pipeline: super::pipeline::Pipeline,
    ) -> Self {
        Self {
            inner: Arc::new(InnerRobotsTxtManager::new(
                config,
                transport,
                body_reader,
                pipeline,
            )),
        }
    }

    #[deprecated(note = "set network_policy and use new")]
    pub fn new_public(config: &CrawlerConfig) -> Self {
        let mut config = config.clone();
        config.network_policy = CrawlNetworkPolicy::PublicOnly;
        Self::new(&config)
    }

    #[cfg(any(test, feature = "crawler-bench", feature = "integration-fixtures"))]
    pub fn with_runtime(config: &CrawlerConfig, runtime: Arc<super::runtime::Runtime>) -> Self {
        let body_reader = BodyReader::new(config.body.clone()).expect("invalid body configuration");
        Self::with_runtime_reader(config, runtime, body_reader)
    }

    #[cfg(any(test, feature = "crawler-bench", feature = "integration-fixtures"))]
    pub(crate) fn with_runtime_reader(
        config: &CrawlerConfig,
        runtime: Arc<super::runtime::Runtime>,
        body_reader: BodyReader,
    ) -> Self {
        Self::with_runtime_pipeline_reader(
            config,
            runtime,
            body_reader,
            super::pipeline::Pipeline::new(config.pipeline.clone())
                .expect("invalid pipeline configuration"),
        )
    }

    #[cfg(any(test, feature = "crawler-bench", feature = "integration-fixtures"))]
    pub(crate) fn with_runtime_pipeline_reader(
        config: &CrawlerConfig,
        runtime: Arc<super::runtime::Runtime>,
        body_reader: BodyReader,
        pipeline: super::pipeline::Pipeline,
    ) -> Self {
        let transport = SharedTransport::new(config, config.transport.clone())
            .expect("invalid crawler transport configuration");
        let mut inner = InnerRobotsTxtManager::new(config, transport, body_reader, pipeline);
        inner.scheduler = Scheduler::new(config, runtime.clock.clone());
        inner.runtime = Some(runtime);
        Self {
            inner: Arc::new(inner),
        }
    }
}

impl RobotsTxtManager {
    pub fn scheduler(&self) -> &Scheduler {
        &self.inner.scheduler
    }
    pub fn stats(&self) -> Stats {
        let counters = &self.inner.counters;
        Stats {
            hits: counters.hits.load(Ordering::Relaxed),
            misses: counters.misses.load(Ordering::Relaxed),
            single_flight_waits: counters.waits.load(Ordering::Relaxed),
            downloads: counters.downloads.load(Ordering::Relaxed),
            evictions: counters.evictions.load(Ordering::Relaxed),
            capacity_deferred: counters.capacity_deferred.load(Ordering::Relaxed),
            unreachable: counters.unreachable.load(Ordering::Relaxed),
            unavailable: counters.unavailable.load(Ordering::Relaxed),
            disallowed: counters.disallowed.load(Ordering::Relaxed),
            entries: self.inner.cache.lock().unwrap().len(),
            capacity: self.inner.config.robots.cache_entries,
            ttl_seconds: self.inner.cache_expiration.as_secs(),
            failure_ttl_seconds: self.inner.failure_ttl().as_secs(),
        }
    }
    pub fn retry_after_ms(&self, url: &Url) -> u64 {
        self.inner
            .cache
            .lock()
            .unwrap()
            .get(&url.origin().ascii_serialization())
            .and_then(|entry| entry.value.as_ref())
            .map_or(1000, |value| {
                value
                    .ttl
                    .saturating_sub(self.inner.now().saturating_duration_since(value.last_check))
                    .as_millis()
                    .max(1) as u64
            })
    }
    pub(crate) fn page_admitted(&self, url: &Url) {
        self.inner.consumed(url);
    }
    pub async fn access(&self, url: &Url) -> RobotAccess {
        self.inner.access(url).await
    }

    pub async fn is_allowed(&self, url: &Url) -> bool {
        self.access(url).await == RobotAccess::Allowed
    }

    pub async fn crawl_delay(&self, url: &Url) -> Option<Duration> {
        self.inner.crawl_delay(url).await
    }

    pub async fn sitemaps(&self, url: &Url) -> Vec<Url> {
        self.inner.sitemaps(url).await
    }

    #[cfg(test)]
    pub fn insert(&self, site: String, robots_txt: robotstxt::Robots) {
        self.inner.insert(site, robots_txt);
    }
}

#[cfg(test)]
mod transport_integration {
    include!("robot_client_transport_tests.rs");
}

#[cfg(test)]
#[path = "robots_origin_tests.rs"]
mod origin_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crawler::{
        robot_client::RobotClient,
        runtime::{Clock, Http, IoFuture, Metrics, Runtime},
        Error,
    };
    use std::{collections::VecDeque, time::Instant};

    type RobotsTxt = robotstxt::Robots;

    #[tokio::test]
    #[allow(deprecated)]
    async fn legacy_public_manager_alias_preserves_public_policy() {
        let mut config = CrawlerConfig::for_tests();
        config.network_policy = CrawlNetworkPolicy::AllowPrivate;
        let manager = RobotsTxtManager::new_public(&config);
        assert_eq!(
            manager.inner.config.network_policy,
            CrawlNetworkPolicy::PublicOnly
        );
        assert_eq!(
            manager
                .access(&Url::parse("http://127.0.0.1/").unwrap())
                .await,
            RobotAccess::Disallowed
        );
    }

    pub(super) struct Reply {
        pub(super) status: u16,
        pub(super) location: Option<&'static str>,
        pub(super) body: &'static str,
    }

    pub(super) struct ScriptedHttp {
        pub(super) replies: Mutex<VecDeque<Reply>>,
        pub(super) requests: Mutex<Vec<String>>,
    }

    impl Http for ScriptedHttp {
        fn send(
            &self,
            url: Url,
            _: Option<Duration>,
        ) -> IoFuture<'_, super::super::Result<reqwest::Response>> {
            Box::pin(async move {
                self.requests.lock().unwrap().push(url.to_string());
                tokio::task::yield_now().await;
                let reply = self.replies.lock().unwrap().pop_front().unwrap();
                let mut response = http_02::Response::builder().status(reply.status);
                if let Some(location) = reply.location {
                    response = response.header("location", location);
                }
                Ok(response.body(reply.body).unwrap().into())
            })
        }
    }

    struct RecordingClock(Mutex<Vec<Duration>>, Instant, Mutex<Duration>);
    impl Default for RecordingClock {
        fn default() -> Self {
            Self(
                Mutex::new(Vec::new()),
                Instant::now(),
                Mutex::new(Duration::ZERO),
            )
        }
    }

    impl Clock for RecordingClock {
        fn now(&self) -> Instant {
            self.1 + *self.2.lock().unwrap()
        }
        fn utc_now(&self) -> chrono::DateTime<chrono::Utc> {
            chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
        }
        fn sleep(&self, duration: Duration) -> IoFuture<'_, ()> {
            Box::pin(async move {
                self.0.lock().unwrap().push(duration);
                *self.2.lock().unwrap() += duration;
            })
        }
    }

    fn scripted_client(
        replies: Vec<Reply>,
    ) -> (RobotClient, Arc<ScriptedHttp>, Arc<RecordingClock>) {
        let mut config = CrawlerConfig::for_tests();
        config.min_crawl_delay_ms = 17;
        config.start_politeness_factor = 0;
        let http = Arc::new(ScriptedHttp {
            replies: Mutex::new(replies.into()),
            requests: Mutex::new(Vec::new()),
        });
        let clock = Arc::new(RecordingClock::default());
        let client = RobotClient::with_runtime(
            &config,
            Arc::new(Runtime {
                http: http.clone(),
                clock: clock.clone(),
                metrics: Arc::new(Metrics::default()),
            }),
        )
        .unwrap();
        (client, http, clock)
    }

    #[tokio::test]
    async fn robots_redirects_without_mime_follow_relative_and_cross_origin_urls() {
        let (client, http, clock) = scripted_client(vec![
            Reply {
                status: 302,
                location: Some("/policies"),
                body: "",
            },
            Reply {
                status: 307,
                location: Some("https://redirect.test/rules"),
                body: "",
            },
            Reply {
                status: 200,
                location: None,
                body: "User-agent: *\nDisallow: /blocked\n",
            },
        ]);
        let url = Url::parse("https://fixture.test/blocked").unwrap();
        assert_eq!(
            client.robots_txt_manager().access(&url).await,
            RobotAccess::Disallowed
        );
        assert_eq!(
            *http.requests.lock().unwrap(),
            [
                "https://fixture.test/robots.txt",
                "https://fixture.test/policies",
                "https://redirect.test/rules",
            ]
        );
        assert_eq!(*clock.0.lock().unwrap(), vec![Duration::from_millis(17)]);
        assert!(matches!(client.get(url).await, Err(Error::DisallowedPath)));
    }

    #[tokio::test]
    async fn robots_redirect_to_private_destination_is_never_sent() {
        let (client, http, _) = scripted_client(vec![
            Reply {
                status: 302,
                location: Some("http://127.0.0.1/private"),
                body: "",
            },
            Reply {
                status: 302,
                location: Some("http://127.0.0.1/private"),
                body: "",
            },
        ]);
        let url = Url::parse("https://fixture.test/page").unwrap();
        assert!(matches!(
            client.get(url).await,
            Err(Error::RobotsUnavailable)
        ));
        assert_eq!(
            *http.requests.lock().unwrap(),
            ["https://fixture.test/robots.txt",]
        );
    }

    #[tokio::test]
    async fn inaccessible_robots_are_distinct_from_disallowed_paths() {
        let (client, http, _) = scripted_client(vec![
            Reply {
                status: 503,
                location: None,
                body: "unavailable",
            },
            Reply {
                status: 503,
                location: None,
                body: "unavailable",
            },
        ]);
        let url = Url::parse("https://fixture.test/page").unwrap();
        assert_eq!(
            client.robots_txt_manager().access(&url).await,
            RobotAccess::Unreachable
        );
        assert!(matches!(
            client.get(url).await,
            Err(Error::RobotsUnavailable)
        ));
        assert_eq!(http.requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn unreachable_robots_are_rechecked_after_cache_expiry() {
        let cached = CheckedRobotsTxt {
            robots: Lookup::Unreachable,
            last_check: std::time::Instant::now() - Duration::from_secs(61),
            ttl: Duration::from_secs(60),
        };
        assert!(cached.is_expired(&Duration::from_secs(60)));
        assert!(!cached.is_expired(&Duration::from_secs(120)));
    }

    #[test]
    fn simple() {
        let ua_token = "AstianGOBot";
        let robots_txt = RobotsTxt::parse(
            ua_token,
            r#"User-agent: AstianGOBot
            Disallow: /test"#,
        )
        .unwrap();

        assert!(!robots_txt.is_allowed(&Url::parse("http://example.com/test").unwrap()));
        assert!(robots_txt.is_allowed(&Url::parse("http://example.com/example").unwrap()));
    }

    #[test]
    fn lowercase() {
        let ua_token = "AstianGOBot";
        let robots_txt = RobotsTxt::parse(
            ua_token,
            r#"User-agent: astiangobot
            Disallow: /test"#,
        )
        .unwrap();

        assert!(!robots_txt.is_allowed(&Url::parse("http://example.com/test").unwrap()));
        assert!(robots_txt.is_allowed(&Url::parse("http://example.com/example").unwrap()));
    }

    #[test]
    fn test_extra_newline() {
        let ua_token = "AstianGOBot";
        let robots_txt = RobotsTxt::parse(
            ua_token,
            r#"User-agent: AstianGOBot


            Disallow: /test"#,
        )
        .unwrap();

        assert!(!robots_txt.is_allowed(&Url::parse("http://example.com/test").unwrap()));
        assert!(robots_txt.is_allowed(&Url::parse("http://example.com/example").unwrap()));
    }

    #[test]
    fn test_multiple_agents() {
        let ua_token = "AstianGOBot";

        let robots_txt = RobotsTxt::parse(
            ua_token,
            r#"User-Agent: GoogleBot
User-Agent: AstianGOBot
Disallow: /

User-Agent: *
Allow: /"#,
        )
        .unwrap();

        assert!(!robots_txt.is_allowed(&Url::parse("http://example.com/test").unwrap()));

        let ua_token = "AstianGOBot";

        let robots_txt = RobotsTxt::parse(
            ua_token,
            r#"User-Agent: GoogleBot, AstianGOBot
Disallow: /

User-Agent: *
Allow: /"#,
        )
        .unwrap();

        assert!(!robots_txt.is_allowed(&Url::parse("http://example.com/test").unwrap()));
    }

    #[test]
    fn test_sitemap() {
        let ua_token = "AstianGOBot";
        let robots_txt = RobotsTxt::parse(
            ua_token,
            r#"User-agent: *
Disallow: /test

Sitemap: http://example.com/sitemap.xml"#,
        )
        .unwrap();

        assert_eq!(robots_txt.sitemaps(), &["http://example.com/sitemap.xml"]);

        let robots_txt = RobotsTxt::parse(
            ua_token,
            r#"User-agent: *
Disallow: /test

SiTeMaP: http://example.com/sitemap.xml"#,
        )
        .unwrap();

        assert_eq!(robots_txt.sitemaps(), &["http://example.com/sitemap.xml"]);
    }

    #[test]
    fn wildcard() {
        let ua_token = "AstianGOBot";

        let robots_txt = RobotsTxt::parse(
            ua_token,
            r#"User-agent: AstianGOBot
Disallow: /test/*
"#,
        )
        .unwrap();

        assert!(!robots_txt.is_allowed(&Url::parse("http://example.com/test/").unwrap()));
        assert!(!robots_txt.is_allowed(&Url::parse("http://example.com/test/foo").unwrap()));
        assert!(!robots_txt.is_allowed(&Url::parse("http://example.com/test/foo/bar").unwrap()));
        assert!(robots_txt.is_allowed(&Url::parse("http://example.com/test").unwrap()));
        assert!(robots_txt.is_allowed(&Url::parse("http://example.com/testfoo").unwrap()));

        let robots_txt = RobotsTxt::parse(
            ua_token,
            r#"User-agent: AstianGOBot
    Disallow: /test/*/bar
    "#,
        )
        .unwrap();

        assert!(robots_txt.is_allowed(&Url::parse("http://example.com/test/").unwrap()));
        assert!(robots_txt.is_allowed(&Url::parse("http://example.com/test/foo").unwrap()));
        assert!(!robots_txt.is_allowed(&Url::parse("http://example.com/test/foo/bar").unwrap()));
        assert!(!robots_txt.is_allowed(&Url::parse("http://example.com/test/foo/baz/bar").unwrap()));
        assert!(robots_txt.is_allowed(&Url::parse("http://example.com/test").unwrap()));
        assert!(robots_txt.is_allowed(&Url::parse("http://example.com/testfoo").unwrap()));
    }
}
