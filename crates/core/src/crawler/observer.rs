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

#[cfg(test)]
mod tests {
    use super::*;

    fn observer() -> (tempfile::TempDir, Observer) {
        let directory = tempfile::tempdir().unwrap();
        let now = Utc::now().to_rfc3339();
        let observer = Observer::new(
            directory.path().join("status.json"),
            Snapshot {
                schema_version: 1,
                run_id: "observer-c02-test".into(),
                pid: std::process::id(),
                started_at: now.clone(),
                updated_at: now,
                finished_at: None,
                phase: "running".into(),
                user_agent: "test".into(),
                workers: 1,
                min_crawl_delay_ms: 1,
                pages_per_domain: 5,
                continuous: false,
                frontier: FrontierState::default(),
                counters: Counters::default(),
                sites: Vec::new(),
                recent_events: Vec::new(),
                warc: Warc {
                    directory: directory.path().display().to_string(),
                    files: 0,
                    bytes: 0,
                    current_file: None,
                },
                history: Vec::new(),
            },
        )
        .unwrap();
        (directory, observer)
    }

    fn datum(url: url::Url) -> super::super::CrawlDatum {
        super::super::CrawlDatum {
            url,
            payload_type: crate::warc::PayloadType::Html,
            body: "<html>saved</html>".into(),
            fetch_time_ms: 3,
            date: Utc::now(),
        }
    }

    #[test]
    fn transport_and_explicit_outcomes_remain_separate() {
        let (directory, observer) = observer();
        let url = url::Url::parse("https://example.test/page").unwrap();
        let domain = Domain::from(&url);
        observer.register_site(domain.as_str().into(), 5);
        observer.event(&domain, Some(&url), "fetching", "candidate");
        for _ in 0..3 {
            observer.http_attempt(&domain, &url);
        }
        observer.redirect(&domain, &url);
        observer.saved(&domain, &datum(url.clone()), "saved.warc.gz".into(), 12);
        for kind in [
            "failed",
            "skipped",
            "deferred",
            "cancelled",
            "storage_failed",
        ] {
            observer.event(&domain, Some(&url), kind, kind);
        }
        observer.check().unwrap();
        let snapshot: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("status.json")).unwrap())
                .unwrap();
        let counters = &snapshot["counters"];
        assert_eq!(counters["attempted"], 1);
        assert_eq!(counters["page_http_attempts"], 3);
        assert_eq!(counters["redirect_hops"], 1);
        assert_eq!(counters["downloaded"], 1);
        assert_eq!(counters["useful_pages"], 1);
        for kind in [
            "failed",
            "skipped",
            "deferred",
            "cancelled",
            "storage_failed",
        ] {
            assert_eq!(counters[kind], 1);
            assert_eq!(snapshot["sites"][0][kind], 1);
        }
    }

    #[test]
    fn finish_preserves_incomplete_outcomes_and_unknown_saved_domains_are_safe() {
        let (_directory, observer) = observer();
        for status in ["deferred", "cancelled", "storage_failed"] {
            let url = url::Url::parse(&format!("https://{status}.test/page")).unwrap();
            let domain = Domain::from(&url);
            observer.register_site(domain.as_str().into(), 5);
            observer.event(&domain, Some(&url), &format!("site_{status}"), status);
        }
        let url = url::Url::parse("https://unregistered.test/page").unwrap();
        observer.saved(
            &Domain::from(&url),
            &datum(url),
            "redirect.warc.gz".into(),
            12,
        );
        observer.finish("stopped");
        observer.check().unwrap();
        let snapshot = observer.snapshot.lock().unwrap();
        assert_eq!(snapshot.counters.downloaded, 1);
        assert_eq!(snapshot.counters.useful_pages, 1);
        assert_eq!(snapshot.counters.completed_sites, 0);
        for (site, expected) in
            snapshot
                .sites
                .iter()
                .zip(["deferred", "cancelled", "storage_failed"])
        {
            assert_eq!(site.status, expected);
            assert!(site.current_url.is_none());
        }
    }
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
    pub page_http_attempts: u64,
    pub redirect_hops: u64,
    pub useful_pages: u64,
    pub deferred: u64,
    pub cancelled: u64,
    pub storage_failed: u64,
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
    pub page_http_attempts: u64,
    pub redirect_hops: u64,
    pub useful_pages: u64,
    pub deferred: u64,
    pub cancelled: u64,
    pub storage_failed: u64,
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
            page_http_attempts: 0,
            redirect_hops: 0,
            useful_pages: 0,
            deferred: 0,
            cancelled: 0,
            storage_failed: 0,
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
            let mut was_completed = false;
            if let Some(index) = s.sites.iter().position(|site| site.host == host) {
                was_completed = s.sites.remove(index).status == "completed";
            }
            if first_visit {
                s.counters.sites += 1;
            } else if was_completed {
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
                if !matches!(
                    site.status.as_str(),
                    "completed" | "failed" | "deferred" | "cancelled" | "storage_failed"
                ) {
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
                "http_attempt" => {
                    site.page_http_attempts += 1;
                    s.counters.page_http_attempts += 1;
                    site.current_url = url.map(ToString::to_string);
                    site.last_http_status = None;
                }
                "redirect" => {
                    site.redirect_hops += 1;
                    s.counters.redirect_hops += 1;
                }
                "deferred" => {
                    site.deferred += 1;
                    s.counters.deferred += 1;
                    site.last_error = Some(message.into());
                }
                "cancelled" => {
                    site.cancelled += 1;
                    s.counters.cancelled += 1;
                    site.last_error = Some(message.into());
                }
                "storage_failed" => {
                    site.storage_failed += 1;
                    s.counters.storage_failed += 1;
                    site.last_error = Some(message.into());
                }
                "site_deferred" | "site_cancelled" | "site_storage_failed" => {
                    site.status = kind.trim_start_matches("site_").into();
                    site.current_url = None;
                    site.next_request_at = None;
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
                    level: if matches!(kind, "failed" | "storage_failed" | "site_storage_failed") {
                        "error"
                    } else if matches!(
                        kind,
                        "skipped" | "deferred" | "cancelled" | "site_deferred" | "site_cancelled"
                    ) {
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

    pub fn http_attempt(&self, domain: &Domain, url: &url::Url) {
        self.event(
            domain,
            Some(url),
            "http_attempt",
            "Intento HTTP de página o sitemap",
        );
    }

    pub fn redirect(&self, domain: &Domain, url: &url::Url) {
        self.event(
            domain,
            Some(url),
            "redirect",
            "Salto de redirección seguido",
        );
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
            let site = s.sites.iter_mut().find(|site| site.host == domain.as_str());
            let http_status = site.as_ref().and_then(|site| site.last_http_status);
            if let Some(site) = site {
                site.pages_downloaded += 1;
                site.useful_pages += 1;
                site.bytes += bytes;
                site.last_fetch_ms = Some(datum.fetch_time_ms);
                site.last_activity_at = now.clone();
                site.last_error = None;
            }
            s.counters.downloaded += 1;
            s.counters.useful_pages += 1;
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
                    http_status,
                    bytes: Some(bytes),
                    fetch_time_ms: Some(datum.fetch_time_ms),
                    warc_file: Some(file),
                },
            );
            s.recent_events.truncate(100);
        });
    }
}
