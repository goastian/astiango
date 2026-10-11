//! Persisted observations from real crawler work. No estimates or generated samples.

use std::{collections::HashSet, fs, io::Write, path::PathBuf, sync::Mutex};

use chrono::Utc;
use serde::Serialize;

use super::Domain;

#[derive(Serialize)]
pub struct Snapshot {
    pub schema_version: u32,
    pub run_id: String,
    pub pid: u32,
    pub started_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
    pub phase: String,
    pub user_agent: String,
    pub workers: usize,
    pub min_crawl_delay_ms: u64,
    pub pages_per_domain: u64,
    pub continuous: bool,
    pub frontier: FrontierState,
    pub counters: Counters,
    pub sites: Vec<Site>,
    pub recent_events: Vec<Event>,
    pub warc: Warc,
    pub history: Vec<Sample>,
}

#[derive(Default, Serialize)]
pub struct FrontierState {
    pub pending_domains: usize,
    pub discovered_domains: usize,
    pub capacity: usize,
}

#[derive(Default, Serialize)]
pub struct Counters {
    pub downloaded: u64,
    pub failed: u64,
    pub skipped: u64,
    pub attempted: u64,
    pub bytes: u64,
    pub sites: usize,
    pub completed_sites: usize,
}

#[derive(Serialize)]
pub struct Site {
    pub host: String,
    pub status: String,
    pub current_url: Option<String>,
    pub pages_downloaded: u64,
    pub failed: u64,
    pub skipped: u64,
    pub attempted: u64,
    pub budget: u64,
    pub bytes: u64,
    pub last_activity_at: String,
    pub last_http_status: Option<u16>,
    pub last_error: Option<String>,
    pub last_fetch_ms: Option<u64>,
    pub next_request_at: Option<String>,
}

impl Site {
    pub fn new(host: String, budget: u64) -> Self {
        Self {
            host,
            status: "queued".into(),
            current_url: None,
            pages_downloaded: 0,
            failed: 0,
            skipped: 0,
            attempted: 0,
            budget,
            bytes: 0,
            last_activity_at: Utc::now().to_rfc3339(),
            last_http_status: None,
            last_error: None,
            last_fetch_ms: None,
            next_request_at: None,
        }
    }
}

#[derive(Serialize)]
pub struct Event {
    pub at: String,
    pub level: String,
    pub kind: String,
    pub message: String,
    pub host: String,
    pub url: Option<String>,
    pub http_status: Option<u16>,
    pub bytes: Option<u64>,
    pub fetch_time_ms: Option<u64>,
    pub warc_file: Option<String>,
}

#[derive(Serialize)]
pub struct Warc {
    pub directory: String,
    pub files: u64,
    pub bytes: u64,
    pub current_file: Option<String>,
}

#[derive(Serialize)]
pub struct Sample {
    pub at: String,
    pub downloaded: u64,
    pub failed: u64,
    pub bytes: u64,
}

pub struct Observer {
    path: PathBuf,
    metrics: Option<std::sync::Arc<super::runtime::Metrics>>,
    snapshot: Mutex<Snapshot>,
    persistence_error: Mutex<Option<String>>,
    registered_domains: Mutex<HashSet<String>>,
}

impl Observer {
    pub fn new(path: PathBuf, snapshot: Snapshot) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let observer = Self {
            path,
            metrics: None,
            snapshot: Mutex::new(snapshot),
            persistence_error: Mutex::new(None),
            registered_domains: Mutex::new(HashSet::new()),
        };
        observer.persist(&observer.snapshot.lock().unwrap())?;
        Ok(observer)
    }

    #[cfg(feature = "crawler-bench")]
    pub(super) fn with_metrics(mut self, metrics: std::sync::Arc<super::runtime::Metrics>) -> Self {
        self.metrics = Some(metrics);
        self
    }

    fn persist(&self, snapshot: &Snapshot) -> anyhow::Result<()> {
        let tmp = self.path.with_extension("json.tmp");
        let mut file = fs::File::create(&tmp)?;
        serde_json::to_writer(&mut file, snapshot)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(tmp, &self.path)?;
        Ok(())
    }

    fn update(&self, f: impl FnOnce(&mut Snapshot)) {
        let _timer = super::runtime::StageTimer::optional(self.metrics.as_ref(), "observer");
        let mut snapshot = self.snapshot.lock().unwrap();
        f(&mut snapshot);
        snapshot.updated_at = Utc::now().to_rfc3339();
        if let Err(error) = self.persist(&snapshot) {
            tracing::error!("Cannot persist crawler observation: {error}");
            *self.persistence_error.lock().unwrap() = Some(error.to_string());
        }
    }

    pub fn check(&self) -> anyhow::Result<()> {
        if let Some(error) = self.persistence_error.lock().unwrap().as_ref() {
            anyhow::bail!("Cannot persist crawler observation: {error}");
        }
        Ok(())
    }

    pub fn heartbeat(&self) {
        self.update(|s| {
            s.history.push(Sample {
                at: Utc::now().to_rfc3339(),
                downloaded: s.counters.downloaded,
                failed: s.counters.failed,
                bytes: s.counters.bytes,
            });
            if s.history.len() > 720 {
                s.history.remove(0);
            }
        });
    }

    pub fn register_site(&self, host: String, budget: u64) {
        let first_visit = self.registered_domains.lock().unwrap().insert(host.clone());
        self.update(|s| {
            if let Some(index) = s.sites.iter().position(|site| site.host == host) {
                s.sites.remove(index);
            }
            if first_visit {
                s.counters.sites += 1;
            } else {
                s.counters.completed_sites = s.counters.completed_sites.saturating_sub(1);
            }
            // Keep the live snapshot bounded; global counters remain cumulative.
            if s.sites.len() >= 500 {
                if let Some(index) = s.sites.iter().position(|site| site.status == "completed") {
                    s.sites.remove(index);
                }
            }
            s.sites.push(Site::new(host, budget));
        });
    }

    pub fn frontier(&self, pending_domains: usize, discovered_domains: usize, capacity: usize) {
        self.update(|s| {
            s.frontier = FrontierState {
                pending_domains,
                discovered_domains,
                capacity,
            };
        });
    }

    pub fn phase(&self, phase: &str) {
        self.update(|s| s.phase = phase.into());
    }

    pub fn finish(&self, phase: &str) {
        self.update(|s| {
            s.phase = phase.into();
            s.finished_at = Some(Utc::now().to_rfc3339());
            for site in &mut s.sites {
                site.current_url = None;
                site.next_request_at = None;
                if site.status != "completed" {
                    site.status = phase.into();
                }
            }
        });
    }

    pub fn event(&self, domain: &Domain, url: Option<&url::Url>, kind: &str, message: &str) {
        self.update(|s| {
            let now = Utc::now().to_rfc3339();
            let site = s
                .sites
                .iter_mut()
                .find(|site| site.host == domain.as_str())
                .unwrap();
            site.last_activity_at = now.clone();
            match kind {
                "site_started" => site.status = "crawling".into(),
                "checking" => {
                    site.status = "crawling".into();
                    site.current_url = url.map(ToString::to_string);
                    site.next_request_at = None;
                }
                "fetching" => {
                    site.status = "crawling".into();
                    site.current_url = url.map(ToString::to_string);
                    site.next_request_at = None;
                    site.last_http_status = None;
                    site.attempted += 1;
                    s.counters.attempted += 1;
                }
                "failed" => {
                    site.failed += 1;
                    site.last_error = Some(message.into());
                    s.counters.failed += 1;
                }
                "skipped" => {
                    site.skipped += 1;
                    s.counters.skipped += 1;
                }
                "site_completed" => {
                    site.status = "completed".into();
                    site.current_url = None;
                    site.next_request_at = None;
                    s.counters.completed_sites += 1;
                }
                _ => {}
            }
            if kind == "checking" {
                return;
            }
            s.recent_events.insert(
                0,
                Event {
                    at: now,
                    level: if kind == "failed" {
                        "error"
                    } else if kind == "skipped" {
                        "warn"
                    } else {
                        "info"
                    }
                    .into(),
                    kind: kind.into(),
                    message: message.into(),
                    host: domain.as_str().into(),
                    url: url.map(ToString::to_string),
                    http_status: site.last_http_status,
                    bytes: None,
                    fetch_time_ms: None,
                    warc_file: None,
                },
            );
            s.recent_events.truncate(100);
        });
    }

    pub fn response(&self, domain: &Domain, status: u16) {
        self.update(|s| {
            let site = s
                .sites
                .iter_mut()
                .find(|site| site.host == domain.as_str())
                .unwrap();
            site.last_http_status = Some(status);
        });
    }

    pub fn waiting(&self, domain: &Domain, duration: std::time::Duration) {
        self.update(|s| {
            let site = s
                .sites
                .iter_mut()
                .find(|site| site.host == domain.as_str())
                .unwrap();
            site.status = "waiting".into();
            site.next_request_at = Some(
                (Utc::now() + chrono::Duration::milliseconds(duration.as_millis() as i64))
                    .to_rfc3339(),
            );
        });
    }

    pub fn saved(
        &self,
        domain: &Domain,
        datum: &super::CrawlDatum,
        file: String,
        compressed_bytes: u64,
    ) {
        self.update(|s| {
            let now = Utc::now().to_rfc3339();
            let bytes = datum.body.len() as u64;
            let site = s
                .sites
                .iter_mut()
                .find(|site| site.host == domain.as_str())
                .unwrap();
            site.pages_downloaded += 1;
            site.bytes += bytes;
            site.last_fetch_ms = Some(datum.fetch_time_ms);
            site.last_activity_at = now.clone();
            site.last_error = None;
            s.counters.downloaded += 1;
            s.counters.bytes += bytes;
            s.warc.files += 1;
            s.warc.bytes += compressed_bytes;
            s.warc.current_file = Some(file.clone());
            s.recent_events.insert(
                0,
                Event {
                    at: now,
                    level: "info".into(),
                    kind: "saved".into(),
                    message: "Página guardada en WARC".into(),
                    host: domain.as_str().into(),
                    url: Some(datum.url.to_string()),
                    http_status: site.last_http_status,
                    bytes: Some(bytes),
                    fetch_time_ms: Some(datum.fetch_time_ms),
                    warc_file: Some(file),
                },
            );
            s.recent_events.truncate(100);
        });
    }
}
