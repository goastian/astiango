use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FetchOutcome {
    DurableSuccess {
        final_url: Url,
    },
    Skipped {
        reason: String,
    },
    PermanentFailure {
        reason: String,
    },
    Deferred {
        reason: String,
        retry_after_ms: Option<u64>,
    },
    Cancelled,
    StorageFailure {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UrlOutcome {
    pub url: Url,
    pub outcome: FetchOutcome,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrawlCounters {
    /// Page and sitemap sends, including HTTPS fallback and retries; excludes robots requests.
    pub page_http_attempts: u64,
    pub redirect_hops: u64,
    pub useful_pages: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    #[default]
    Completed,
    Deferred,
    Cancelled,
    StorageFailed,
}

#[must_use = "inspect job outcomes so deferred and storage failures cannot become silent success"]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobOutcome {
    pub status: JobStatus,
    pub counters: CrawlCounters,
    pub urls: Vec<UrlOutcome>,
}

impl JobOutcome {
    pub fn is_success(&self) -> bool {
        self.status == JobStatus::Completed
            && self.urls.iter().all(|entry| {
                matches!(
                    entry.outcome,
                    FetchOutcome::DurableSuccess { .. } | FetchOutcome::Skipped { .. }
                )
            })
    }

    pub(crate) fn record(&mut self, url: Url, outcome: FetchOutcome) {
        match outcome {
            FetchOutcome::DurableSuccess { .. } => self.counters.useful_pages += 1,
            FetchOutcome::StorageFailure { .. } => self.status = JobStatus::StorageFailed,
            FetchOutcome::Cancelled if self.status != JobStatus::StorageFailed => {
                self.status = JobStatus::Cancelled;
            }
            FetchOutcome::Deferred { .. } if self.status == JobStatus::Completed => {
                self.status = JobStatus::Deferred;
            }
            _ => {}
        }
        self.urls.push(UrlOutcome { url, outcome });
    }
}
