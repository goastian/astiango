use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub enabled: bool,
    pub max_depth: usize,
    pub calendar_family_budget: u64,
    pub query_family_budget: u64,
    pub session_family_budget: u64,
    pub repeated_content_budget: u64,
    pub review_sample_every: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: true,
            max_depth: 32,
            calendar_family_budget: 256,
            query_family_budget: 256,
            session_family_budget: 64,
            repeated_content_budget: 64,
            review_sample_every: 64,
        }
    }
}
impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_depth > 0
                && self.calendar_family_budget > 0
                && self.query_family_budget > 0
                && self.session_family_budget > 0
                && self.repeated_content_budget > 0
                && self.review_sample_every > 0,
            "invalid trap budgets"
        );
        Ok(())
    }
    pub fn pattern(&self, url: &Url) -> (String, Option<(&'static str, u64)>) {
        let segments: Vec<_> = url
            .path_segments()
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect();
        let route = segments
            .iter()
            .map(|part| {
                if part.chars().all(|c| c.is_ascii_digit()) {
                    "{n}"
                } else {
                    *part
                }
            })
            .collect::<Vec<_>>()
            .join("/");
        let mut keys = url
            .query_pairs()
            .map(|(key, _)| key.to_string())
            .collect::<Vec<_>>();
        keys.sort();
        keys.dedup();
        let pattern = format!(
            "{}:{route}?{}",
            url.origin().ascii_serialization(),
            keys.join("&")
        );
        if !self.enabled {
            return (pattern, None);
        }
        let policy = if segments.len() > self.max_depth {
            Some(("path_depth", 1))
        } else if keys.iter().any(|k| {
            matches!(
                k.to_ascii_lowercase().as_str(),
                "session" | "sessionid" | "sid" | "phpsessid" | "jsessionid"
            )
        }) {
            Some(("session_family", self.session_family_budget))
        } else if segments.windows(3).any(|s| {
            s[0].len() == 4
                && s[0]
                    .parse::<u32>()
                    .is_ok_and(|v| (1900..=2200).contains(&v))
                && s[1].parse::<u32>().is_ok_and(|v| (1..=12).contains(&v))
                && s[2].parse::<u32>().is_ok_and(|v| (1..=31).contains(&v))
        }) {
            Some(("calendar_family", self.calendar_family_budget))
        } else if keys.len() >= 4
            && keys
                .iter()
                .filter(|k| {
                    matches!(
                        k.to_ascii_lowercase().as_str(),
                        "filter" | "color" | "size" | "brand" | "category" | "sort" | "price"
                    )
                })
                .count()
                >= 2
        {
            Some(("facet_family", self.query_family_budget))
        } else {
            None
        };
        (pattern, policy)
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Family {
    pub admitted: u64,
    pub repeated: u64,
    pub empty: u64,
    pub reviewed: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Quarantine {
    pub url: Url,
    pub reason: String,
    pub pattern: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub review_sample: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legitimate_pagination_signed_urls_and_slashes_keep_coverage() {
        let config = Config::default();
        for i in 0..1000 {
            for url in [
                format!("https://example.org/blog/page/{i}/"),
                format!("https://example.org/article?page={i}&signature=a%2Fb&token=z&expires=123"),
            ] {
                assert!(config.pattern(&Url::parse(&url).unwrap()).1.is_none());
            }
        }
        assert_eq!(
            config
                .pattern(&Url::parse("https://example.org/calendar/2026/10/12").unwrap())
                .1
                .unwrap()
                .0,
            "calendar_family"
        );
    }
}
