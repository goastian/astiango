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

use std::{
    ops::Deref,
    panic,
    sync::{Arc, Mutex},
    time::Duration,
};

use dashmap::DashMap;
use url::Url;

use crate::config::{CrawlNetworkPolicy, CrawlerConfig};

use super::{encoded_body, transport::SharedTransport, Site};

const RETRY_ROBOTSTXT_UNREACHABLE: bool = false;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RobotAccess {
    Allowed,
    Disallowed,
    Unreachable,
}

#[derive(Debug)]
enum Lookup<T> {
    Found(T),
    /// 404
    Unavailable,
    /// 5xx (and other network errors)
    Unreachable,
}

struct CheckedRobotsTxt {
    robots: Lookup<robotstxt::Robots>,
    last_check: std::time::Instant,
}

impl CheckedRobotsTxt {
    #[cfg(test)]
    fn new(robots: Lookup<robotstxt::Robots>) -> Self {
        Self {
            robots,
            last_check: std::time::Instant::now(),
        }
    }
}

impl CheckedRobotsTxt {
    #[cfg(test)]
    fn is_expired(&self, expiration: &Duration) -> bool {
        self.is_expired_at(expiration, std::time::Instant::now())
    }

    fn is_expired_at(&self, expiration: &Duration, now: std::time::Instant) -> bool {
        // A transient DNS or server failure must not block a host forever in
        // the continuous crawler. Keep denying requests until this TTL expires.
        now.duration_since(self.last_check) >= *expiration
    }
}

struct InnerRobotsTxtManager {
    cache: DashMap<Site, CheckedRobotsTxt>,
    last_prune: Arc<Mutex<std::time::Instant>>,
    transport: SharedTransport,
    cache_expiration: Duration,
    user_agent: String,
    min_crawl_delay: Duration,
    max_crawl_delay: Duration,
    config: Arc<CrawlerConfig>,
    runtime: Option<Arc<super::runtime::Runtime>>,
}

impl InnerRobotsTxtManager {
    fn new(config: &CrawlerConfig, transport: SharedTransport) -> Self {
        let cache_expiration = Duration::from_secs(config.robots_txt_cache_sec);
        let user_agent = config.user_agent.token.clone();
        let min_crawl_delay = Duration::from_millis(config.min_crawl_delay_ms);
        let max_crawl_delay = Duration::from_millis(config.max_crawl_delay_ms);

        Self {
            transport,
            cache_expiration,
            last_prune: Arc::new(Mutex::new(std::time::Instant::now())),
            cache: DashMap::new(),
            user_agent: user_agent.to_string(),
            min_crawl_delay,
            max_crawl_delay,
            config: Arc::new(config.clone()),
            runtime: None,
        }
    }

    async fn access(&self, url: &Url) -> RobotAccess {
        if super::robot_client::validate_network_url(self.config.network_policy, url).is_err() {
            return RobotAccess::Disallowed;
        }
        match &self.get(url).await.robots {
            Lookup::Found(robots_txt) if robots_txt.is_allowed(url) => RobotAccess::Allowed,
            Lookup::Found(_) => RobotAccess::Disallowed,
            Lookup::Unavailable => RobotAccess::Allowed,
            Lookup::Unreachable => RobotAccess::Unreachable,
        }
    }

    async fn crawl_delay(&self, url: &Url) -> Option<Duration> {
        match &self.get(url).await.robots {
            Lookup::Found(robots_txt) => robots_txt.crawl_delay(),
            Lookup::Unavailable | Lookup::Unreachable => None,
        }
    }

    async fn sitemaps(&self, url: &Url) -> Vec<Url> {
        match &self.get(url).await.robots {
            Lookup::Found(robots_txt) => robots_txt
                .sitemaps()
                .iter()
                .filter_map(|s| Url::parse(s).ok())
                .collect(),
            Lookup::Unavailable | Lookup::Unreachable => vec![],
        }
    }

    fn now(&self) -> std::time::Instant {
        self.runtime
            .as_ref()
            .map_or_else(std::time::Instant::now, |runtime| runtime.clock.now())
    }

    async fn sleep(&self, duration: Duration) {
        match &self.runtime {
            Some(runtime) => runtime.sleep(duration).await,
            None => tokio::time::sleep(duration).await,
        }
    }

    async fn body(&self, response: reqwest::Response) -> super::Result<String> {
        match &self.runtime {
            Some(runtime) => runtime.body(response).await,
            None => encoded_body(response).await,
        }
    }

    async fn request(&self, url: Url) -> super::Result<super::robot_client::RequestBuilder> {
        super::robot_client::validate_network_url(self.config.network_policy, &url)?;
        if let Some(runtime) = &self.runtime {
            return Ok(super::robot_client::RequestBuilder::injected(
                runtime.clone(),
                url,
            ));
        }
        Ok(super::robot_client::RequestBuilder::managed(
            self.transport.clone(),
            url,
        ))
    }

    async fn fetch_robots_txt_from_url(&self, initial_url: &str) -> Lookup<robotstxt::Robots> {
        let Ok(mut url) = Url::parse(initial_url) else {
            return Lookup::Unreachable;
        };
        for _ in 0..=5 {
            let request = match self.request(url.clone()).await {
                Ok(request) => request,
                Err(_) => return Lookup::Unreachable,
            };
            let response = request.send().await;
            // Robots requests and their redirects also obey the configured delay.
            self.sleep(self.min_crawl_delay).await;
            let Ok(response) = response else {
                return Lookup::Unreachable;
            };
            if matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                let next = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|location| url.join(location).ok());
                let Some(next) = next else {
                    return Lookup::Unreachable;
                };
                url = next;
                continue;
            }
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Lookup::Unavailable;
            }
            if response.status() != reqwest::StatusCode::OK {
                return Lookup::Unreachable;
            }
            let Ok(body) = self.body(response).await else {
                return Lookup::Unreachable;
            };
            let user_agent = self.user_agent.clone();
            return match panic::catch_unwind(|| robotstxt::Robots::parse(&user_agent, &body)) {
                Ok(Ok(robots)) => Lookup::Found(robots),
                _ => Lookup::Unreachable,
            };
        }
        Lookup::Unreachable
    }

    async fn fetch_robots_txt_without_retry(&self, site: &Site) -> Lookup<robotstxt::Robots> {
        match self
            .fetch_robots_txt_from_url(&format!("https://{}/robots.txt", site.0))
            .await
        {
            Lookup::Unreachable => {
                self.fetch_robots_txt_from_url(&format!("http://{}/robots.txt", site.0))
                    .await
            }
            Lookup::Unavailable => {
                match self
                    .fetch_robots_txt_from_url(&format!("http://{}/robots.txt", site.0))
                    .await
                {
                    Lookup::Found(robots_txt) => Lookup::Found(robots_txt),
                    Lookup::Unreachable => Lookup::Unavailable,
                    Lookup::Unavailable => Lookup::Unavailable,
                }
            }
            Lookup::Found(robots_txt) => Lookup::Found(robots_txt),
        }
    }

    async fn fetch_robots_txt_with_retry(&self, site: &Site) -> Lookup<robotstxt::Robots> {
        for _ in 0..3 {
            match self.fetch_robots_txt_without_retry(site).await {
                Lookup::Found(robots_txt) => return Lookup::Found(robots_txt),
                Lookup::Unavailable => return Lookup::Unavailable,
                Lookup::Unreachable => {}
            }

            self.sleep(self.max_crawl_delay).await;
        }

        Lookup::Unreachable
    }

    async fn fetch_robots_txt(&self, site: &Site) -> CheckedRobotsTxt {
        let robots = if RETRY_ROBOTSTXT_UNREACHABLE {
            self.fetch_robots_txt_with_retry(site).await
        } else {
            self.fetch_robots_txt_without_retry(site).await
        };
        CheckedRobotsTxt {
            robots,
            last_check: self.now(),
        }
    }

    fn maybe_prune(&self) {
        if self.now().duration_since(*self.last_prune.lock().unwrap()) < Duration::from_secs(60) {
            return;
        }

        self.cache
            .retain(|_, v| !v.is_expired_at(&self.cache_expiration, self.now()));

        *self.last_prune.lock().unwrap() = self.now();
    }

    async fn get(&self, url: &Url) -> impl Deref<Target = CheckedRobotsTxt> + '_ {
        self.maybe_prune();
        let site = Site(url.host_str().unwrap_or_default().to_string());

        let cache_should_update = self
            .cache
            .get_mut(&site)
            .map(|v| v.is_expired_at(&self.cache_expiration, self.now()))
            .unwrap_or(true);

        if cache_should_update {
            self.cache
                .insert(site.clone(), self.fetch_robots_txt(&site).await);
        }

        self.cache.get(&site).unwrap()
    }

    #[cfg(test)]
    fn insert(&self, site: String, robots_txt: robotstxt::Robots) {
        self.cache
            .insert(Site(site), CheckedRobotsTxt::new(Lookup::Found(robots_txt)));
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
        Self {
            inner: Arc::new(InnerRobotsTxtManager::new(config, transport)),
        }
    }

    #[deprecated(note = "set network_policy and use new")]
    pub fn new_public(config: &CrawlerConfig) -> Self {
        let mut config = config.clone();
        config.network_policy = CrawlNetworkPolicy::PublicOnly;
        Self::new(&config)
    }

    #[cfg(any(test, feature = "crawler-bench"))]
    pub fn with_runtime(config: &CrawlerConfig, runtime: Arc<super::runtime::Runtime>) -> Self {
        let transport = SharedTransport::new(config, config.transport.clone())
            .expect("invalid crawler transport configuration");
        let mut inner = InnerRobotsTxtManager::new(config, transport);
        inner.last_prune = Arc::new(Mutex::new(runtime.clock.now()));
        inner.runtime = Some(runtime);
        Self {
            inner: Arc::new(inner),
        }
    }
}

impl RobotsTxtManager {
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

    struct Reply {
        status: u16,
        location: Option<&'static str>,
        body: &'static str,
    }

    struct ScriptedHttp {
        replies: Mutex<VecDeque<Reply>>,
        requests: Mutex<Vec<String>>,
    }

    impl Http for ScriptedHttp {
        fn send(
            &self,
            url: Url,
            _: Option<Duration>,
        ) -> IoFuture<'_, super::super::Result<reqwest::Response>> {
            Box::pin(async move {
                self.requests.lock().unwrap().push(url.to_string());
                let reply = self.replies.lock().unwrap().pop_front().unwrap();
                let mut response = http_02::Response::builder().status(reply.status);
                if let Some(location) = reply.location {
                    response = response.header("location", location);
                }
                Ok(response.body(reply.body).unwrap().into())
            })
        }
    }

    #[derive(Default)]
    struct RecordingClock(Mutex<Vec<Duration>>);

    impl Clock for RecordingClock {
        fn now(&self) -> Instant {
            Instant::now()
        }
        fn utc_now(&self) -> chrono::DateTime<chrono::Utc> {
            chrono::DateTime::from_timestamp(1_700_000_000, 0).unwrap()
        }
        fn sleep(&self, duration: Duration) -> IoFuture<'_, ()> {
            Box::pin(async move { self.0.lock().unwrap().push(duration) })
        }
    }

    fn scripted_client(
        replies: Vec<Reply>,
    ) -> (RobotClient, Arc<ScriptedHttp>, Arc<RecordingClock>) {
        let mut config = CrawlerConfig::for_tests();
        config.min_crawl_delay_ms = 17;
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
        assert_eq!(*clock.0.lock().unwrap(), vec![Duration::from_millis(17); 3]);
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
            [
                "https://fixture.test/robots.txt",
                "http://fixture.test/robots.txt",
            ]
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
        assert_eq!(http.requests.lock().unwrap().len(), 2);
    }

    #[test]
    fn unreachable_robots_are_rechecked_after_cache_expiry() {
        let cached = CheckedRobotsTxt {
            robots: Lookup::Unreachable,
            last_check: std::time::Instant::now() - Duration::from_secs(61),
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
