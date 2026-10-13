use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub file: Option<std::path::PathBuf>,
    pub centrality_weight: f64,
    pub novelty_weight: f64,
    pub freshness_weight: f64,
    pub cost_weight: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            file: None,
            centrality_weight: 1.0,
            novelty_weight: 0.25,
            freshness_weight: 0.25,
            cost_weight: 1.0,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Signal {
    pub url: url::Url,
    pub novelty: f64,
    pub freshness: f64,
    pub cost: f64,
    pub source: String,
    pub observed_at: chrono::DateTime<chrono::Utc>,
}
impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            [
                self.centrality_weight,
                self.novelty_weight,
                self.freshness_weight,
                self.cost_weight
            ]
            .iter()
            .all(|n| n.is_finite() && *n >= 0.0),
            "invalid planner signal weights"
        );
        Ok(())
    }
    pub fn score(&self, centrality: f64, signal: Option<&Signal>) -> f64 {
        let (novelty, freshness, cost) =
            signal.map_or((1.0, 0.0, 1.0), |s| (s.novelty, s.freshness, s.cost));
        (centrality.max(0.0) * self.centrality_weight
            + novelty * self.novelty_weight
            + freshness * self.freshness_weight)
            / (1.0 + self.cost_weight * cost.max(0.0))
    }
}
