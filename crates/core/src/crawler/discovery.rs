use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub cycle_ms: u64,
    pub max_depth: u32,
    pub max_documents: u32,
    pub max_urls: u64,
    pub max_total_bytes: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            cycle_ms: 3_600_000,
            max_depth: 8,
            max_documents: 64,
            max_urls: 100_000,
            max_total_bytes: 64 * 1024 * 1024,
        }
    }
}
impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.cycle_ms > 0
                && (1..=64).contains(&self.max_depth)
                && self.max_documents > 0
                && self.max_urls > 0
                && self.max_total_bytes > 0,
            "invalid discovery quotas"
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Cycle {
    pub round: u64,
    pub documents: u32,
    pub urls: u64,
    pub bytes: u64,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub url: Url,
    pub source: Url,
    pub discovered_at: chrono::DateTime<chrono::Utc>,
    pub lastmod: Option<chrono::DateTime<chrono::Utc>>,
    pub sitemap: bool,
    pub depth: u32,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub candidates: u64,
    pub reason: String,
}
#[derive(Debug, thiserror::Error)]
#[error("discovery storage failed: {0}")]
pub struct StorageError(pub anyhow::Error);
