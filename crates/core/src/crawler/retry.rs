use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub max_attempts: u32,
    pub max_elapsed_ms: u64,
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    pub circuit_failures: u32,
    pub circuit_open_ms: u64,
    pub origin_attempts_per_minute: u32,
    pub global_attempts_per_minute: u32,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            max_attempts: 6,
            max_elapsed_ms: 86_400_000,
            initial_backoff_ms: 1000,
            max_backoff_ms: 900_000,
            circuit_failures: 3,
            circuit_open_ms: 60_000,
            origin_attempts_per_minute: 32,
            global_attempts_per_minute: 1024,
        }
    }
}
impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_attempts > 0
                && self.max_attempts <= 100
                && self.max_elapsed_ms > 0
                && self.initial_backoff_ms > 0
                && self.max_backoff_ms >= self.initial_backoff_ms
                && self.circuit_failures > 0
                && self.circuit_open_ms > 0
                && self.origin_attempts_per_minute > 0
                && self.global_attempts_per_minute > 0,
            "invalid retry budgets"
        );
        Ok(())
    }
    pub fn delay(&self, failures: u32, remote_minimum: u64, identity: &str) -> u64 {
        let base = self
            .initial_backoff_ms
            .saturating_mul(1u64 << failures.saturating_sub(1).min(30))
            .min(self.max_backoff_ms);
        let seed = bloom::fast_stable_hash_64(format!("{identity}:{failures}").as_bytes());
        let jitter = seed % (base / 4 + 1);
        remote_minimum.max(base.saturating_add(jitter))
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UrlState {
    pub attempts: u32,
    pub failures: u32,
    pub first_attempt_ms: u64,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OriginState {
    pub consecutive_failures: u32,
    pub penalty_ms: u64,
    pub not_before_ms: u64,
    pub window_start_ms: u64,
    pub window_attempts: u32,
}
impl OriginState {
    pub fn failure(&mut self, config: &Config, now: u64, delay: u64) -> bool {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        self.penalty_ms = self.penalty_ms.max(delay);
        let open = self.consecutive_failures >= config.circuit_failures;
        let delay = if open {
            delay.max(config.circuit_open_ms)
        } else {
            delay
        };
        self.not_before_ms = self.not_before_ms.max(now.saturating_add(delay));
        open
    }
    pub fn success(&mut self, now: u64) {
        self.consecutive_failures = 0;
        self.penalty_ms /= 2;
        self.not_before_ms = now.saturating_add(self.penalty_ms);
    }
}
pub fn retry_after(
    headers: &reqwest::header::HeaderMap,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<u64> {
    let value = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    value
        .parse::<u64>()
        .ok()
        .map(|n| n.saturating_mul(1000))
        .or_else(|| {
            chrono::DateTime::parse_from_rfc2822(value)
                .ok()
                .map(|date| {
                    (date.with_timezone(&chrono::Utc) - now)
                        .num_milliseconds()
                        .max(0) as u64
                })
        })
}
pub fn tls_validation_failure(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        let message = cause.to_string().to_ascii_lowercase();
        message.contains("certificate")
            || message.contains("unknownissuer")
            || message.contains("certnotvalidforname")
    })
}
pub fn is_transient(reason: &str) -> bool {
    !matches!(
        reason,
        "origin_not_eligible"
            | "origin_retry_wait"
            | "global_attempt_budget"
            | "origin_attempt_budget"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn positive_jitter_never_shortens_remote_minimum_and_recovery_is_finite() {
        let config = Config::default();
        for n in 1..100 {
            assert!(config.delay(n, 3_600_000, "signed-url") >= 3_600_000);
        }
        let mut origin = OriginState::default();
        assert!(!origin.failure(&config, 100, 1000));
        origin.failure(&config, 200, 2000);
        assert!(origin.failure(&config, 300, 4000));
        assert!(origin.not_before_ms >= 60_300);
        for n in 0..20 {
            origin.success(100_000 + n);
        }
        assert_eq!(origin.penalty_ms, 0);
        assert_eq!(origin.consecutive_failures, 0);
    }
}

#[cfg(test)]
mod header_tests {
    use super::*;
    #[test]
    fn retry_after_accepts_delta_and_http_date_without_shortening_the_minimum() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-12T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("retry-after", "3600".parse().unwrap());
        assert_eq!(retry_after(&headers, now), Some(3_600_000));
        headers.insert(
            "retry-after",
            "Mon, 12 Oct 2026 01:00:00 GMT".parse().unwrap(),
        );
        assert_eq!(retry_after(&headers, now), Some(3_600_000));
        headers.insert("retry-after", "nonsense".parse().unwrap());
        assert_eq!(retry_after(&headers, now), None);
    }
}
