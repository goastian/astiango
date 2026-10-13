use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub min_interval_ms: u64,
    pub initial_interval_ms: u64,
    pub max_interval_ms: u64,
    pub full_check_every: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            min_interval_ms: 60_000,
            initial_interval_ms: 3_600_000,
            max_interval_ms: 604_800_000,
            full_check_every: 20,
        }
    }
}
impl Config {
    pub fn bounds(
        &self,
        value: f64,
        mime: Option<&str>,
        elapsed_ms: u64,
        bytes: usize,
    ) -> (u64, u64) {
        let value = if value.is_finite() {
            value.clamp(0.25, 4.0)
        } else {
            1.0
        };
        let class = match mime
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "application/rss+xml" | "application/atom+xml" => 0.5,
            "application/xml" | "text/xml" => 2.0,
            _ => 1.0,
        };
        let cost = (1.0 + elapsed_ms as f64 / 30_000.0 + bytes as f64 / 8_388_608.0).min(4.0);
        let interval = |base| {
            ((base as f64 * class * cost / value).round() as u64)
                .clamp(self.min_interval_ms, self.max_interval_ms)
        };
        (
            interval(self.initial_interval_ms),
            interval(self.max_interval_ms),
        )
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.min_interval_ms > 0
                && self.min_interval_ms <= self.initial_interval_ms
                && self.initial_interval_ms <= self.max_interval_ms
                && self.full_check_every > 0,
            "invalid recrawl intervals"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct PayloadReference {
    pub version: u32,
    pub fetch_id: String,
    pub segment: String,
    pub record_offset: u64,
    pub record_end: u64,
    pub payload_sha256: String,
    pub payload_bytes: usize,
    pub url: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Resource {
    #[serde(default)]
    pub mime: Option<String>,
    pub reference: PayloadReference,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub robots: crate::capture::Robots,
    pub checks: u64,
    pub changes: u64,
    pub last_checked_ms: u64,
    pub next_eligible_ms: u64,
    pub interval_ms: u64,
}
impl Resource {
    pub fn headers(&self, config: &Config, destination: &url::Url) -> reqwest::header::HeaderMap {
        let mut headers = reqwest::header::HeaderMap::new();
        if !config.enabled
            || destination.as_str() != self.reference.url
            || self.checks % config.full_check_every == 0
        {
            return headers;
        }
        if let Some(etag) = &self.etag {
            if let Ok(value) = etag.parse() {
                headers.insert(reqwest::header::IF_NONE_MATCH, value);
            }
        } else if let Some(date) = &self.last_modified {
            if let Ok(value) = date.parse() {
                headers.insert(reqwest::header::IF_MODIFIED_SINCE, value);
            }
        }
        headers
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recrawl_value_cost_and_class_preserve_freshness_and_fixed_profile_bounds() {
        let config = Config::default();
        let html = config.bounds(1.0, Some("text/html"), 0, 0);
        assert!(config.bounds(4.0, Some("text/html"), 0, 0).0 < html.0);
        assert!(config.bounds(1.0, Some("text/html"), 30_000, 8_388_608).0 > html.0);
        assert!(
            config
                .bounds(1.0, Some("application/rss+xml; charset=utf-8"), 0, 0)
                .0
                < html.0
        );
        assert!(config.bounds(1.0, Some("application/xml"), 0, 0).0 > html.0);
        let fixed = Config {
            min_interval_ms: 1,
            initial_interval_ms: 1,
            max_interval_ms: 1,
            ..Default::default()
        };
        assert_eq!(
            fixed.bounds(4.0, Some("application/rss+xml"), 100_000, 32_000_000),
            (1, 1)
        );
        assert_eq!(config.bounds(f64::NAN, None, 0, 0), html);
    }
    #[test]
    fn validators_are_scoped_to_the_exact_final_url_and_full_checks_are_sampled() {
        let url: url::Url = "https://example.org/a?signature=keep".parse().unwrap();
        let mut resource = Resource {
            mime: Some("text/html".into()),
            reference: PayloadReference {
                version: 1,
                fetch_id: uuid::Uuid::new_v4().to_string(),
                segment: "saved.warc.gz".into(),
                record_offset: 0,
                record_end: 100,
                payload_sha256: "a".repeat(64),
                payload_bytes: 40,
                url: url.to_string(),
            },
            etag: Some("\"v1\"".into()),
            last_modified: Some("Mon, 12 Oct 2026 00:00:00 GMT".into()),
            robots: Default::default(),
            checks: 1,
            changes: 0,
            last_checked_ms: 0,
            next_eligible_ms: 1,
            interval_ms: 1,
        };
        let config = Config::default();
        assert!(resource
            .headers(&config, &url)
            .contains_key("if-none-match"));
        assert!(resource
            .headers(
                &config,
                &"https://other.example/a?signature=keep".parse().unwrap()
            )
            .is_empty());
        resource.checks = 20;
        assert!(resource.headers(&config, &url).is_empty());
        resource.checks = 1;
        resource.etag = None;
        assert!(resource
            .headers(&config, &url)
            .contains_key("if-modified-since"));
        resource.last_modified = None;
        assert!(resource.headers(&config, &url).is_empty());
    }
}
