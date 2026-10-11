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

use crate::{config::CrawlerConfig, crawler};

use super::{encoded_body, Site};

const RETRY_ROBOTSTXT_UNREACHABLE: bool = false;

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
    client: reqwest::Client,
    cache_expiration: Duration,
    user_agent: String,
    min_crawl_delay: Duration,
    max_crawl_delay: Duration,
    public_config: Option<Arc<CrawlerConfig>>,
    runtime: Option<Arc<super::runtime::Runtime>>,
}

impl InnerRobotsTxtManager {
    fn new(config: &CrawlerConfig) -> Self {
        let client = crawler::robot_client::reqwest_client(config).unwrap();
        let cache_expiration = Duration::from_secs(config.robots_txt_cache_sec);
        let user_agent = config.user_agent.token.clone();
        let min_crawl_delay = Duration::from_millis(config.min_crawl_delay_ms);
        let max_crawl_delay = Duration::from_millis(config.max_crawl_delay_ms);

        Self {
            client,
            cache_expiration,
            last_prune: Arc::new(Mutex::new(std::time::Instant::now())),
            cache: DashMap::new(),
            user_agent: user_agent.to_string(),
            min_crawl_delay,
            max_crawl_delay,
            public_config: None,
            runtime: None,
        }
    }

    async fn is_allowed(&self, url: &Url) -> bool {
        if self.public_config.is_some() && super::robot_client::validate_public_url(url).is_err() {
            return false;
        }
        match &self.get(url).await.robots {
            Lookup::Found(robots_txt) => robots_txt.is_allowed(url),
            Lookup::Unavailable => true,
            Lookup::Unreachable => false,
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

    async fn fetch_injected_robots(
        &self,
        runtime: &Arc<super::runtime::Runtime>,
        initial: &str,
    ) -> Lookup<robotstxt::Robots> {
        let Ok(mut url) = Url::parse(initial) else {
            return Lookup::Unreachable;
        };
        for _ in 0..=5 {
            let response =
                super::robot_client::RequestBuilder::injected(runtime.clone(), url.clone())
                    .send()
                    .await;
            self.sleep(self.min_crawl_delay).await;
            let Ok(response) = response else {
                return Lookup::Unreachable;
            };
            if response.status().is_redirection() {
                let next = response
                    .headers()
                    .get(reqwest::header::LOCATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| url.join(v).ok());
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

    async fn fetch_robots_txt_from_url(&self, url: &str) -> Lookup<robotstxt::Robots> {
        if let Some(runtime) = &self.runtime {
            return self.fetch_injected_robots(runtime, url).await;
        }
        let request = if let Some(config) = &self.public_config {
            return self.fetch_public_robots(config, url).await;
        } else {
            self.client.get(url)
        };
        let res = match super::robot_client::RequestBuilder::new(request)
            .timeout(Duration::from_secs(60))
            .send()
            .await
        {
            Ok(res) => {
                if res.status() != reqwest::StatusCode::OK {
                    match res.status() {
                        reqwest::StatusCode::NOT_FOUND => return Lookup::Unavailable,
                        _ => return Lookup::Unreachable,
                    }
                }

                let body = match self.body(res).await {
                    Ok(body) => body,
                    Err(_) => return Lookup::Unreachable,
                };

                let self_user_agent = self.user_agent.clone();
                match panic::catch_unwind(|| robotstxt::Robots::parse(&self_user_agent, &body)) {
                    Ok(Ok(r)) => Lookup::Found(r),
                    _ => Lookup::Unreachable,
                }
            }
            Err(_) => Lookup::Unreachable,
        };

        self.sleep(self.min_crawl_delay).await;

        res
    }

    async fn fetch_public_robots(
        &self,
        config: &CrawlerConfig,
        initial_url: &str,
    ) -> Lookup<robotstxt::Robots> {
        let Ok(mut url) = Url::parse(initial_url) else {
            return Lookup::Unreachable;
        };
        for _ in 0..=5 {
            let request = match super::robot_client::public_request(config, &url).await {
                Ok(request) => request,
                Err(_) => return Lookup::Unreachable,
            };
            let response = super::robot_client::RequestBuilder::new(request)
                .send()
                .await;
            // Robots requests and their redirects also obey the configured delay.
            self.sleep(self.min_crawl_delay).await;
            let Ok(response) = response else {
                return Lookup::Unreachable;
            };
            if response.status().is_redirection() {
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
        Self {
            inner: Arc::new(InnerRobotsTxtManager::new(config)),
        }
    }

    #[cfg(any(test, feature = "crawler-bench"))]
    pub fn with_runtime(config: &CrawlerConfig, runtime: Arc<super::runtime::Runtime>) -> Self {
        let mut inner = InnerRobotsTxtManager::new(config);
        inner.last_prune = Arc::new(Mutex::new(runtime.clock.now()));
        inner.runtime = Some(runtime);
        Self {
            inner: Arc::new(inner),
        }
    }

    pub fn new_public(config: &CrawlerConfig) -> Self {
        let mut inner = InnerRobotsTxtManager::new(config);
        inner.public_config = Some(Arc::new(config.clone()));
        Self {
            inner: Arc::new(inner),
        }
    }
}

impl RobotsTxtManager {
    pub async fn is_allowed(&self, url: &Url) -> bool {
        self.inner.is_allowed(url).await
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
mod tests {
    use super::*;

    type RobotsTxt = robotstxt::Robots;

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
