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
    transport::SharedTransport,
    Error, Result,
};
use crate::config::{CrawlNetworkPolicy, CrawlerConfig};
use crate::crawler::robots_txt::{RobotAccess, RobotsTxtManager};
use anyhow::anyhow;
use std::{net::IpAddr, sync::Arc, time::Duration};
use url::Url;

pub(super) fn client_builder(config: &CrawlerConfig) -> reqwest::ClientBuilder {
    let timeout = Duration::from_secs(config.timeout_seconds);

    let mut headers = reqwest::header::HeaderMap::default();
    headers.insert(
        reqwest::header::ACCEPT,
        reqwest::header::HeaderValue::from_static("text/html"),
    );
    headers.insert(
        reqwest::header::ACCEPT_ENCODING,
        reqwest::header::HeaderValue::from_static("identity"),
    );
    headers.insert(
        reqwest::header::ACCEPT_LANGUAGE,
        reqwest::header::HeaderValue::from_static("en-US,en;q=0.9,*;q=0.8"),
    );

    reqwest::Client::builder()
        .timeout(timeout)
        .connect_timeout(timeout)
        .no_gzip()
        .no_brotli()
        .no_deflate()
        .http2_keep_alive_interval(None)
        .default_headers(headers)
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(&config.user_agent.full)
}

/// Fail closed for non-public addresses, including IPv4 encoded as IPv6.
pub(super) fn is_public_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_broadcast()
                || ip.is_documentation()
                || a == 0
                || a >= 240
                || (a == 100 && (64..=127).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 192 && b == 88 && c == 99)
                || (a == 198 && (b == 18 || b == 19)))
        }
        IpAddr::V6(ip) => {
            if let Some(v4) = ip.to_ipv4_mapped() {
                return is_public_address(IpAddr::V4(v4));
            }
            let s = ip.segments();
            // Global unicast only; exclude transition, protocol and documentation ranges.
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && (s[1] < 0x0200 || s[1] == 0x0db8))
                && s[0] != 0x2002
                && !(s[0] == 0x3fff && s[1] < 0x1000)
        }
    }
}

pub(super) fn validate_public_url(url: &Url) -> Result<()> {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.port_or_known_default(), Some(80 | 443))
    {
        return Err(Error::NonPublicDestination(
            "crawl destination must be public HTTP(S) on port 80 or 443".into(),
        ));
    }
    match url.host() {
        Some(url::Host::Domain(host)) if host.contains('.') && host != "localhost" => Ok(()),
        Some(url::Host::Ipv4(ip)) if is_public_address(IpAddr::V4(ip)) => Ok(()),
        Some(url::Host::Ipv6(ip)) if is_public_address(IpAddr::V6(ip)) => Ok(()),
        _ => Err(Error::NonPublicDestination(
            "non-public crawl destination".into(),
        )),
    }
}

pub(super) fn validate_network_url(policy: CrawlNetworkPolicy, url: &Url) -> Result<()> {
    match policy {
        CrawlNetworkPolicy::PublicOnly => validate_public_url(url),
        CrawlNetworkPolicy::AllowPrivate => {
            if matches!(url.scheme(), "http" | "https")
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
            {
                Ok(())
            } else {
                Err(Error::NonPublicDestination(
                    "crawl destination must be HTTP(S) without credentials".into(),
                ))
            }
        }
    }
}

pub fn validate_seed_urls(config: &CrawlerConfig, seeds: &[Url]) -> Result<()> {
    for seed in seeds {
        validate_network_url(config.network_policy, seed)?;
    }
    Ok(())
}

#[cfg(test)]
fn validate_public_addresses(host: &str, addresses: &[std::net::SocketAddr]) -> Result<()> {
    if addresses.is_empty() || addresses.iter().any(|addr| !is_public_address(addr.ip())) {
        return Err(Error::NonPublicDestination(format!(
            "DNS returned a non-public or empty destination for {host}"
        )));
    }
    Ok(())
}

/// Reqwest client that respects robots.txt for each request.
#[derive(Clone)]
pub struct RobotClient {
    robots_txt_manager: RobotsTxtManager,
    transport: SharedTransport,
    body_reader: BodyReader,
    extractor: super::extraction::Extractor,
    config: Arc<CrawlerConfig>,
    runtime: Option<Arc<super::runtime::Runtime>>,
}

impl RobotClient {
    pub fn new(config: &CrawlerConfig) -> Result<Self> {
        let transport = SharedTransport::new(config, config.transport.clone())?;
        let body_reader = BodyReader::new(config.body.clone())?;
        Ok(Self {
            runtime: None,
            robots_txt_manager: RobotsTxtManager::with_readers(
                config,
                transport.clone(),
                body_reader.clone(),
            ),
            transport,
            body_reader,
            extractor: super::extraction::Extractor::default(),
            config: Arc::new(config.clone()),
        })
    }

    #[deprecated(note = "use new with network_policy and validate_seed_urls")]
    pub async fn new_public(config: &CrawlerConfig, seeds: &[Url]) -> Result<Self> {
        if config.network_policy != CrawlNetworkPolicy::PublicOnly {
            return Err(Error::NonPublicDestination(
                "new_public requires network_policy = public_only".into(),
            ));
        }
        validate_seed_urls(config, seeds)?;
        Self::new(config)
    }

    #[cfg(any(test, feature = "crawler-bench"))]
    pub fn with_runtime(
        config: &CrawlerConfig,
        runtime: Arc<super::runtime::Runtime>,
    ) -> Result<Self> {
        let body_reader = BodyReader::new(config.body.clone())?;
        Ok(Self {
            transport: SharedTransport::new(config, config.transport.clone())?,
            robots_txt_manager: RobotsTxtManager::with_runtime_reader(
                config,
                runtime.clone(),
                body_reader.clone(),
            ),
            body_reader,
            extractor: super::extraction::Extractor::default(),
            config: Arc::new(config.clone()),
            runtime: Some(runtime),
        })
    }

    pub(crate) fn metrics(&self) -> Option<&Arc<super::runtime::Metrics>> {
        self.runtime.as_ref().map(|runtime| &runtime.metrics)
    }

    pub(crate) fn transport(&self) -> &SharedTransport {
        &self.transport
    }

    #[cfg(any(test, feature = "crawler-bench"))]
    pub fn with_transport(config: &CrawlerConfig, transport: SharedTransport) -> Self {
        let body_reader = BodyReader::new(config.body.clone()).expect("invalid body configuration");
        Self {
            robots_txt_manager: RobotsTxtManager::with_readers(
                config,
                transport.clone(),
                body_reader.clone(),
            ),
            transport,
            body_reader,
            extractor: super::extraction::Extractor::default(),
            config: Arc::new(config.clone()),
            runtime: None,
        }
    }

    pub(crate) fn now(&self) -> std::time::Instant {
        self.runtime
            .as_ref()
            .map_or_else(std::time::Instant::now, |runtime| runtime.clock.now())
    }

    pub(crate) fn utc_now(&self) -> chrono::DateTime<chrono::Utc> {
        self.runtime
            .as_ref()
            .map_or_else(chrono::Utc::now, |runtime| runtime.clock.utc_now())
    }

    pub(crate) async fn sleep(&self, duration: Duration) {
        match &self.runtime {
            Some(runtime) => runtime.sleep(duration).await,
            None => tokio::time::sleep(duration).await,
        }
    }

    pub(crate) fn body_reader(&self) -> &BodyReader {
        &self.body_reader
    }

    pub(crate) fn extractor(&self) -> &super::extraction::Extractor {
        &self.extractor
    }

    pub(crate) async fn body(
        &self,
        response: reqwest::Response,
        kind: BodyKind,
    ) -> Result<AdmittedBody> {
        let _timer = super::runtime::StageTimer::optional(self.metrics(), "http_body");
        self.body_reader
            .read(response, kind)
            .await
            .map_err(Error::from)
    }

    pub fn robots_txt_manager(&self) -> &RobotsTxtManager {
        &self.robots_txt_manager
    }

    pub async fn get(&self, url: Url) -> Result<RequestBuilder> {
        validate_network_url(self.config.network_policy, &url)?;
        match self.robots_txt_manager.access(&url).await {
            RobotAccess::Allowed => {}
            RobotAccess::Disallowed => return Err(Error::DisallowedPath),
            RobotAccess::Unreachable => return Err(Error::RobotsUnavailable),
        }

        if let Some(runtime) = &self.runtime {
            return Ok(RequestBuilder::injected(runtime.clone(), url, &self.config));
        }
        Ok(RequestBuilder::managed(self.transport.clone(), url))
    }
}

pub struct RequestBuilder {
    request: Option<reqwest::RequestBuilder>,
    injected: Option<(
        Arc<super::runtime::Runtime>,
        Url,
        Option<Duration>,
        Duration,
        Duration,
    )>,
    managed: Option<(SharedTransport, Url, Option<Duration>)>,
}

impl From<reqwest::RequestBuilder> for RequestBuilder {
    fn from(builder: reqwest::RequestBuilder) -> Self {
        Self::new(builder)
    }
}

impl RequestBuilder {
    pub fn new(builder: reqwest::RequestBuilder) -> Self {
        Self {
            request: Some(builder),
            injected: None,
            managed: None,
        }
    }

    pub(crate) fn injected(
        runtime: Arc<super::runtime::Runtime>,
        url: Url,
        config: &CrawlerConfig,
    ) -> Self {
        Self {
            request: None,
            injected: Some((
                runtime,
                url,
                None,
                Duration::from_millis(config.body.header_timeout_ms),
                Duration::from_millis(config.body.total_timeout_ms)
                    .min(Duration::from_secs(config.timeout_seconds)),
            )),
            managed: None,
        }
    }

    pub(crate) fn managed(transport: SharedTransport, url: Url) -> Self {
        Self {
            request: None,
            injected: None,
            managed: Some((transport, url, None)),
        }
    }

    pub async fn send(self) -> Result<reqwest::Response> {
        if let Some((runtime, url, timeout, headers, total)) = self.injected {
            let total = timeout.map_or(total, |timeout| timeout.min(total));
            let started = std::time::Instant::now();
            let mut response =
                tokio::time::timeout(headers.min(total), runtime.http.send(url, Some(total)))
                    .await
                    .map_err(|_| {
                        Error::Body(if headers < total {
                            super::body::BodyError::HeadersTimeout
                        } else {
                            super::body::BodyError::TotalTimeout
                        })
                    })??;
            response
                .extensions_mut()
                .insert(super::body::RequestDeadline {
                    started,
                    expires: started + total,
                });
            return Ok(response);
        }
        if let Some((transport, url, timeout)) = self.managed {
            return transport.send(url, timeout).await;
        }
        #[cfg(not(test))]
        {
            self.request
                .unwrap()
                .send()
                .await
                .map_err(|e| Error::from(anyhow!(e)))
        }
        #[cfg(test)]
        {
            Err(Error::from(anyhow!("Not allowed in tests")))
        }
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        if let Some((_, _, value, _, _)) = &mut self.injected {
            *value = Some(timeout);
        }
        if let Some((_, _, value)) = &mut self.managed {
            *value = Some(timeout);
        }
        self.request = self.request.map(|request| request.timeout(timeout));
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_migration_defaults_to_public_only() {
        let mut legacy = serde_json::to_value(CrawlerConfig::for_tests()).unwrap();
        legacy.as_object_mut().unwrap().remove("network_policy");
        legacy.as_object_mut().unwrap().remove("max_redirect_hops");
        legacy.as_object_mut().unwrap().remove("transport");
        legacy.as_object_mut().unwrap().remove("body");
        let config: CrawlerConfig = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(config.network_policy, CrawlNetworkPolicy::PublicOnly);
        assert_eq!(config.max_redirect_hops, 10);
        assert_eq!(config.transport.max_client_slots, 128);
        assert_eq!(config.transport.max_slots_per_origin, 4);
        assert_eq!(config.body.html.wire_limit_bytes, 32 * 1024 * 1024);

        legacy["network_policy"] = serde_json::json!("allow_private");
        legacy["max_redirect_hops"] = serde_json::json!(3);
        let config: CrawlerConfig = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(config.network_policy, CrawlNetworkPolicy::AllowPrivate);
        assert_eq!(config.max_redirect_hops, 3);
        legacy["network_policy"] = serde_json::json!("legacy");
        assert!(serde_json::from_value::<CrawlerConfig>(legacy).is_err());
    }

    #[test]
    fn private_network_access_requires_explicit_policy() {
        let private = Url::parse("http://127.0.0.1:1234/fixture").unwrap();
        let mut config = CrawlerConfig::for_tests();
        assert!(validate_seed_urls(&config, std::slice::from_ref(&private)).is_err());
        config.network_policy = CrawlNetworkPolicy::AllowPrivate;
        assert!(validate_seed_urls(&config, &[private]).is_ok());
        for url in ["ftp://127.0.0.1/", "http://user:password@127.0.0.1/"] {
            assert!(validate_seed_urls(&config, &[Url::parse(url).unwrap()]).is_err());
        }
    }

    #[test]
    fn public_dns_validation_rejects_empty_and_mixed_destinations() {
        assert!(validate_public_addresses("fixture.test", &[]).is_err());
        let public = "1.1.1.1:0".parse().unwrap();
        let private = "127.0.0.1:0".parse().unwrap();
        assert!(validate_public_addresses("fixture.test", &[public]).is_ok());
        assert!(validate_public_addresses("fixture.test", &[public, private]).is_err());
    }

    #[tokio::test]
    #[allow(deprecated)]
    async fn public_constructor_alias_cannot_override_the_configured_policy() {
        let mut config = CrawlerConfig::for_tests();
        let seed = Url::parse("https://fixture.test/page").unwrap();
        let client = RobotClient::new_public(&config, &[seed]).await.unwrap();
        assert_eq!(client.config.network_policy, CrawlNetworkPolicy::PublicOnly);
        assert!(matches!(
            client.get(Url::parse("http://127.0.0.1/").unwrap()).await,
            Err(Error::NonPublicDestination(_))
        ));
        config.network_policy = CrawlNetworkPolicy::AllowPrivate;
        assert!(RobotClient::new_public(&config, &[]).await.is_err());
    }

    #[test]
    fn public_crawl_rejects_internal_and_special_destinations() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.168.1.1",
            "192.0.2.1",
            "198.18.0.1",
            "224.0.0.1",
            "240.0.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
            "2002:7f00:1::",
        ] {
            assert!(!is_public_address(address.parse().unwrap()), "{address}");
        }
        for address in ["1.1.1.1", "8.8.8.8", "2606:4700:4700::1111"] {
            assert!(is_public_address(address.parse().unwrap()), "{address}");
        }
        for url in [
            "http://localhost/",
            "http://127.0.0.1/",
            "https://example.org:8080/",
            "https://user:password@example.org/",
            "ftp://example.org/",
        ] {
            assert!(
                validate_public_url(&Url::parse(url).unwrap()).is_err(),
                "{url}"
            );
        }
    }

    #[tokio::test]
    async fn test_errs_disallowed_path() {
        let mut config = CrawlerConfig::for_tests();
        config.network_policy = CrawlNetworkPolicy::AllowPrivate;
        let client = RobotClient::new(&config).unwrap();

        let robots_txt =
            robotstxt::Robots::parse("TestBot", "User-agent: *\nDisallow: /test\nAllow: /example")
                .unwrap();

        client
            .robots_txt_manager()
            .insert("example.com".to_string(), robots_txt);

        let url = Url::parse("http://example.com/test").unwrap();
        assert!(client.get(url).await.is_err());

        let url = Url::parse("http://example.com/example").unwrap();
        assert!(client.get(url).await.is_ok());
    }
}
