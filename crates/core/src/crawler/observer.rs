//! Reconstructible crawler observations, projected independently of durable crawl data.

use std::{
    collections::{HashMap, VecDeque},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use chrono::Utc;
use serde::Serialize;

use super::Domain;

#[derive(Clone, Serialize)]
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

#[derive(Clone, Default, Serialize)]
pub struct FrontierState {
    pub pending_domains: usize,
    pub discovered_domains: usize,
    pub capacity: usize,
}

#[derive(Clone, Default, Serialize)]
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

#[derive(Clone, Serialize)]
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

#[derive(Clone, Serialize)]
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

#[derive(Clone, Serialize)]
pub struct Warc {
    pub directory: String,
    pub files: u64,
    pub bytes: u64,
    pub current_file: Option<String>,
}

#[derive(Clone, Serialize)]
pub struct Sample {
    pub at: String,
    pub downloaded: u64,
    pub failed: u64,
    pub bytes: u64,
}

const PROJECTION_INTERVAL: Duration = Duration::from_secs(1);
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);
const EVENT_CAPACITY: usize = 100;
const HISTORY_CAPACITY: usize = 720;
const SITE_CAPACITY: usize = 500;
const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Default, Serialize)]
pub struct ObserverStats {
    pub projection_interval_ms: u64,
    pub event_capacity: usize,
    pub updates: u64,
    pub events_total: u64,
    /// Evictions from the visual ring, including events already published previously.
    pub events_dropped: u64,
    /// Successfully renamed snapshots, not serialization attempts.
    pub snapshot_writes: u64,
    pub write_calls: u64,
    pub bytes_written: u64,
    pub errors: u64,
    pub pending_updates: u64,
    pub last_error: Option<String>,
    pub last_failure_at: Option<String>,
    pub last_published_at: Option<String>,
    pub last_publish_latency_ms: u64,
    pub max_publish_latency_ms: u64,
    pub legacy_write_estimate: u64,
}

#[derive(Default)]
struct Rings {
    events: VecDeque<Event>,
    history: VecDeque<Sample>,
    events_total: u64,
    events_dropped: u64,
    capacity: usize,
    last_sample: Option<Instant>,
}

impl Rings {
    fn event(&mut self, event: Event) {
        self.events_total += 1;
        if self.events.len() >= self.capacity {
            self.events.pop_back();
            self.events_dropped += 1;
        }
        self.events.push_front(event);
    }
}

struct State {
    snapshot: Snapshot,
    rings: Rings,
    // Identity/status metadata outlives the bounded visual site list so totals remain exact.
    registered_domains: HashMap<String, bool>,
    stats: ObserverStats,
    version: u64,
    published_version: u64,
    attempts: u64,
    dirty_since: Option<Instant>,
    force: bool,
    shutdown: bool,
}

#[derive(Default)]
struct IoCounters {
    calls: AtomicU64,
    bytes: AtomicU64,
}

struct CountingWriter<'a> {
    file: fs::File,
    counters: &'a IoCounters,
}

impl Write for CountingWriter<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.counters.calls.fetch_add(1, Ordering::Relaxed);
        let written = self.file.write(buffer)?;
        self.counters
            .bytes
            .fetch_add(written as u64, Ordering::Relaxed);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[derive(Serialize)]
struct Projection {
    #[serde(flatten)]
    snapshot: Snapshot,
    telemetry: ObserverStats,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport: Option<super::transport::TransportStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<super::body::BodyStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    extraction: Option<super::extraction::ExtractionStats>,
}

type Publisher = dyn Fn(&Path, &Projection, &IoCounters, bool) -> anyhow::Result<()> + Send + Sync;

struct Shared {
    state: Mutex<State>,
    wake: Condvar,
    io: IoCounters,
    path: PathBuf,
    interval: Duration,
    publisher: Arc<Publisher>,
    publication: Mutex<()>,
    metrics: Mutex<Option<Arc<super::runtime::Metrics>>>,
    transport: Mutex<Option<super::transport::SharedTransport>>,
    body: Mutex<Option<super::body::BodyReader>>,
    extraction: Mutex<Option<super::extraction::Extractor>>,
}

pub struct Observer {
    shared: Arc<Shared>,
    projector: Mutex<Option<thread::JoinHandle<()>>>,
    // The synchronous implementation is only constructible in tests/the benchmark feature.
    legacy: bool,
}

impl Observer {
    pub fn new(path: PathBuf, snapshot: Snapshot) -> anyhow::Result<Self> {
        Self::create(
            path,
            snapshot,
            PROJECTION_INTERVAL,
            EVENT_CAPACITY,
            false,
            Arc::new(persist),
        )
    }

    #[cfg(any(test, feature = "crawler-bench"))]
    pub(super) fn new_legacy(path: PathBuf, snapshot: Snapshot) -> anyhow::Result<Self> {
        Self::create(
            path,
            snapshot,
            PROJECTION_INTERVAL,
            EVENT_CAPACITY,
            true,
            Arc::new(persist),
        )
    }

    fn create(
        path: PathBuf,
        mut snapshot: Snapshot,
        interval: Duration,
        capacity: usize,
        legacy: bool,
        publisher: Arc<Publisher>,
    ) -> anyhow::Result<Self> {
        let events = std::mem::take(&mut snapshot.recent_events);
        let history = std::mem::take(&mut snapshot.history);
        let mut rings = Rings {
            capacity: capacity.max(1),
            ..Rings::default()
        };
        for event in events.into_iter().rev() {
            rings.event(event);
        }
        rings
            .history
            .extend(history.into_iter().rev().take(HISTORY_CAPACITY));
        rings.history.make_contiguous().reverse();
        snapshot.sites.truncate(SITE_CAPACITY);
        let registered_domains = snapshot
            .sites
            .iter()
            .map(|site| (site.host.clone(), site.status == "completed"))
            .collect();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                snapshot,
                rings,
                registered_domains,
                stats: ObserverStats {
                    projection_interval_ms: interval.as_millis() as u64,
                    event_capacity: capacity.max(1),
                    legacy_write_estimate: 1,
                    ..ObserverStats::default()
                },
                version: 1,
                published_version: 0,
                attempts: 0,
                dirty_since: Some(Instant::now()),
                force: true,
                shutdown: false,
            }),
            wake: Condvar::new(),
            io: IoCounters::default(),
            path,
            interval,
            publisher,
            publication: Mutex::new(()),
            metrics: Mutex::new(None),
            transport: Mutex::new(None),
            body: Mutex::new(None),
            extraction: Mutex::new(None),
        });
        let projector = if legacy {
            publish(&shared, true);
            None
        } else {
            let worker = shared.clone();
            Some(
                thread::Builder::new()
                    .name("crawler-observer".into())
                    .spawn(move || project(worker))?,
            )
        };
        Ok(Self {
            shared,
            projector: Mutex::new(projector),
            legacy,
        })
    }

    #[cfg(feature = "crawler-bench")]
    pub(super) fn with_metrics(self, metrics: Arc<super::runtime::Metrics>) -> Self {
        *self.shared.metrics.lock().unwrap() = Some(metrics);
        self
    }

    pub(super) fn observe_transport(&self, transport: super::transport::SharedTransport) {
        *self.shared.transport.lock().unwrap() = Some(transport);
        self.heartbeat();
    }

    pub(super) fn observe_body(&self, body: super::body::BodyReader) {
        *self.shared.body.lock().unwrap() = Some(body);
        self.heartbeat();
    }

    pub(super) fn observe_extraction(&self, extractor: super::extraction::Extractor) {
        *self.shared.extraction.lock().unwrap() = Some(extractor);
        self.heartbeat();
    }

    fn update(&self, f: impl FnOnce(&mut Snapshot, &mut Rings, &mut HashMap<String, bool>)) {
        let metrics = self.shared.metrics.lock().unwrap().clone();
        let _timer = super::runtime::StageTimer::optional(metrics.as_ref(), "observer");
        {
            let mut state = self.shared.state.lock().unwrap();
            let State {
                snapshot,
                rings,
                registered_domains,
                ..
            } = &mut *state;
            f(snapshot, rings, registered_domains);
            state.snapshot.updated_at = Utc::now().to_rfc3339();
            state.stats.updates += 1;
            state.stats.legacy_write_estimate += 1;
            state.version += 1;
            state.dirty_since.get_or_insert_with(Instant::now);
        }
        if self.legacy {
            publish(&self.shared, true);
        }
    }

    pub fn statistics(&self) -> ObserverStats {
        let state = self.shared.state.lock().unwrap();
        statistics(&self.shared, &state)
    }

    /// Diagnostic only. A telemetry failure must never cancel otherwise durable work.
    pub fn check(&self) -> anyhow::Result<()> {
        if let Some(error) = self.shared.state.lock().unwrap().stats.last_error.as_ref() {
            anyhow::bail!("Cannot persist crawler observation: {error}");
        }
        Ok(())
    }

    /// Force publication outside the fetch path (tests, reports and graceful shutdown).
    pub fn flush(&self) -> anyhow::Result<()> {
        self.flush_timeout(FLUSH_TIMEOUT)
    }

    pub fn flush_timeout(&self, timeout: Duration) -> anyhow::Result<()> {
        if self.legacy {
            return self.check();
        }
        let started = Instant::now();
        let mut state = self.shared.state.lock().unwrap();
        let target = state.version;
        let attempts = state.attempts;
        state.force = true;
        self.shared.wake.notify_one();
        loop {
            if state.published_version >= target {
                return Ok(());
            }
            if state.attempts > attempts {
                if let Some(error) = &state.stats.last_error {
                    anyhow::bail!("Cannot persist crawler observation: {error}");
                }
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                anyhow::bail!("Timed out waiting for crawler observation publication");
            }
            state = self.shared.wake.wait_timeout(state, remaining).unwrap().0;
        }
    }

    pub fn heartbeat(&self) {
        self.update(|s, rings, _| {
            if rings
                .last_sample
                .is_some_and(|sample| sample.elapsed() < Duration::from_secs(5))
            {
                return;
            }
            rings.last_sample = Some(Instant::now());
            if rings.history.len() >= HISTORY_CAPACITY {
                rings.history.pop_front();
            }
            rings.history.push_back(Sample {
                at: Utc::now().to_rfc3339(),
                downloaded: s.counters.downloaded,
                failed: s.counters.failed,
                bytes: s.counters.bytes,
            });
        });
    }

    pub fn register_site(&self, host: String, budget: u64) {
        self.update(|s, _, registered| {
            let previous = registered.insert(host.clone(), false);
            if let Some(index) = s.sites.iter().position(|site| site.host == host) {
                s.sites.remove(index);
            }
            match previous {
                None => s.counters.sites += 1,
                Some(true) => {
                    s.counters.completed_sites = s.counters.completed_sites.saturating_sub(1)
                }
                Some(false) => {}
            }
            if s.sites.len() >= SITE_CAPACITY {
                let index = s
                    .sites
                    .iter()
                    .position(|site| {
                        matches!(
                            site.status.as_str(),
                            "completed" | "failed" | "deferred" | "cancelled" | "storage_failed"
                        )
                    })
                    .unwrap_or(0);
                s.sites.remove(index);
            }
            s.sites.push(Site::new(host, budget));
        });
    }

    pub fn frontier(&self, pending_domains: usize, discovered_domains: usize, capacity: usize) {
        self.update(|s, _, _| {
            s.frontier = FrontierState {
                pending_domains,
                discovered_domains,
                capacity,
            }
        });
    }

    pub fn phase(&self, phase: &str) {
        self.update(|s, _, _| s.phase = phase.into());
    }

    pub fn finish(&self, phase: &str) {
        self.update(|s, _, _| {
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
        if let Err(error) = self.flush() {
            tracing::warn!("Crawler observation shutdown: {error}");
        }
    }

    pub fn event(&self, domain: &Domain, url: Option<&url::Url>, kind: &str, message: &str) {
        self.update(|s, rings, registered| {
            let now = Utc::now().to_rfc3339();
            // Totals are independent of retained visual events and sites.
            match kind {
                "fetching" => s.counters.attempted += 1,
                "failed" => s.counters.failed += 1,
                "skipped" => s.counters.skipped += 1,
                "http_attempt" => s.counters.page_http_attempts += 1,
                "redirect" => s.counters.redirect_hops += 1,
                "deferred" => s.counters.deferred += 1,
                "cancelled" => s.counters.cancelled += 1,
                "storage_failed" => s.counters.storage_failed += 1,
                "site_completed" => {
                    if registered.insert(domain.as_str().into(), true) != Some(true) {
                        s.counters.completed_sites += 1;
                    }
                }
                _ => {}
            }
            let site = s.sites.iter_mut().find(|site| site.host == domain.as_str());
            let mut http_status = None;
            if let Some(site) = site {
                site.last_activity_at = now.clone();
                match kind {
                    "site_started" => site.status = "crawling".into(),
                    "checking" | "fetching" => {
                        site.status = "crawling".into();
                        site.current_url = url.map(|url| bounded(url.as_str()));
                        site.next_request_at = None;
                        if kind == "fetching" {
                            site.last_http_status = None;
                            site.attempted += 1;
                        }
                    }
                    "failed" => {
                        site.failed += 1;
                        site.last_error = Some(bounded(message));
                    }
                    "skipped" => site.skipped += 1,
                    "http_attempt" => {
                        site.page_http_attempts += 1;
                        site.current_url = url.map(|url| bounded(url.as_str()));
                        site.last_http_status = None;
                    }
                    "redirect" => site.redirect_hops += 1,
                    "deferred" => {
                        site.deferred += 1;
                        site.last_error = Some(bounded(message));
                    }
                    "cancelled" => {
                        site.cancelled += 1;
                        site.last_error = Some(bounded(message));
                    }
                    "storage_failed" => {
                        site.storage_failed += 1;
                        site.last_error = Some(bounded(message));
                    }
                    "site_deferred"
                    | "site_cancelled"
                    | "site_storage_failed"
                    | "site_completed" => {
                        site.status = kind.trim_start_matches("site_").into();
                        site.current_url = None;
                        site.next_request_at = None;
                    }
                    _ => {}
                }
                http_status = site.last_http_status;
            }
            if kind == "checking" {
                return;
            }
            rings.event(Event {
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
                kind: bounded(kind),
                message: bounded(message),
                host: domain.as_str().into(),
                url: url.map(|url| bounded(url.as_str())),
                http_status,
                bytes: None,
                fetch_time_ms: None,
                warc_file: None,
            });
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
        self.update(|s, _, _| {
            if let Some(site) = s.sites.iter_mut().find(|site| site.host == domain.as_str()) {
                site.last_http_status = Some(status);
            }
        });
    }

    pub fn waiting(&self, domain: &Domain, duration: Duration) {
        self.update(|s, _, _| {
            if let Some(site) = s.sites.iter_mut().find(|site| site.host == domain.as_str()) {
                site.status = "waiting".into();
                site.next_request_at = Some(
                    chrono::Duration::from_std(duration)
                        .ok()
                        .and_then(|delay| Utc::now().checked_add_signed(delay))
                        .unwrap_or(chrono::DateTime::<Utc>::MAX_UTC)
                        .to_rfc3339(),
                );
            }
        });
    }

    pub fn saved(
        &self,
        domain: &Domain,
        datum: &super::CrawlDatum,
        file: String,
        compressed_bytes: u64,
    ) {
        self.update(|s, rings, _| {
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
            s.warc.current_file = Some(bounded(&file));
            rings.event(Event {
                at: now,
                level: "info".into(),
                kind: "saved".into(),
                message: "Página guardada en WARC".into(),
                host: domain.as_str().into(),
                url: Some(bounded(datum.url.as_str())),
                http_status,
                bytes: Some(bytes),
                fetch_time_ms: Some(datum.fetch_time_ms),
                warc_file: Some(bounded(&file)),
            });
        });
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        // Never join an unresponsive filesystem operation. The final explicit flush is bounded.
        let mut state = self.shared.state.lock().unwrap();
        state.shutdown = true;
        self.shared.wake.notify_one();
        drop(state);
        if let Some(handle) = self.projector.lock().unwrap().take() {
            if handle.is_finished() {
                let _ = handle.join();
            }
        }
    }
}

fn bounded(value: &str) -> String {
    let mut end = value.len().min(8192);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

fn statistics(shared: &Shared, state: &State) -> ObserverStats {
    let mut stats = state.stats.clone();
    stats.events_total = state.rings.events_total;
    stats.events_dropped = state.rings.events_dropped;
    stats.pending_updates = state.version.saturating_sub(state.published_version);
    stats.write_calls = shared.io.calls.load(Ordering::Relaxed);
    stats.bytes_written = shared.io.bytes.load(Ordering::Relaxed);
    stats
}

fn project(shared: Arc<Shared>) {
    let mut next = Instant::now();
    loop {
        let mut state = shared.state.lock().unwrap();
        loop {
            let dirty = state.version > state.published_version;
            if state.shutdown && !dirty {
                return;
            }
            if dirty && (state.force || state.shutdown || Instant::now() >= next) {
                break;
            }
            let delay = next.saturating_duration_since(Instant::now());
            let delay = if delay.is_zero() {
                shared.interval
            } else {
                delay
            };
            state = shared.wake.wait_timeout(state, delay).unwrap().0;
        }
        let shutdown = state.shutdown;
        state.force = false;
        drop(state);
        publish(&shared, false);
        if shutdown {
            return;
        }
        next = Instant::now() + shared.interval;
    }
}

fn publish(shared: &Shared, legacy: bool) {
    let _publication = shared.publication.lock().unwrap();
    let (mut projection, version, dirty_since) = {
        let mut state = shared.state.lock().unwrap();
        let mut snapshot = state.snapshot.clone();
        snapshot.recent_events = state.rings.events.iter().cloned().collect();
        snapshot.history = state.rings.history.iter().cloned().collect();
        let mut telemetry = statistics(shared, &state);
        // This envelope is only visible after its atomic rename succeeds. IO counters
        // describe previously completed publications; statistics() includes this one.
        telemetry.last_published_at = Some(Utc::now().to_rfc3339());
        telemetry.last_error = None;
        telemetry.pending_updates = 0;
        let version = state.version;
        let dirty_since = state.dirty_since.take().unwrap_or_else(Instant::now);
        (
            Projection {
                snapshot,
                telemetry,
                transport: None,
                body: None,
                extraction: None,
            },
            version,
            dirty_since,
        )
    };
    let transport = shared.transport.lock().unwrap().clone();
    projection.transport = transport.as_ref().map(|transport| transport.stats());
    let body = shared.body.lock().unwrap().clone();
    projection.body = body.as_ref().map(|body| body.stats());
    let extraction = shared.extraction.lock().unwrap().clone();
    projection.extraction = extraction.as_ref().map(|extractor| extractor.stats());
    let metrics = shared.metrics.lock().unwrap().clone();
    let _timer = super::runtime::StageTimer::optional(metrics.as_ref(), "observer_projection");
    let result = (shared.publisher)(&shared.path, &projection, &shared.io, legacy);
    let mut state = shared.state.lock().unwrap();
    state.attempts += 1;
    let mut warning = None;
    match result {
        Ok(()) => {
            state.published_version = state.published_version.max(version);
            state.stats.snapshot_writes += 1;
            state.stats.last_error = None;
            state.stats.last_published_at = Some(Utc::now().to_rfc3339());
            state.stats.last_publish_latency_ms = dirty_since.elapsed().as_millis() as u64;
            state.stats.max_publish_latency_ms = state
                .stats
                .max_publish_latency_ms
                .max(state.stats.last_publish_latency_ms);
        }
        Err(error) => {
            let message = bounded(&error.to_string());
            if state.stats.last_error.as_ref() != Some(&message) {
                warning = Some(message.clone());
            }
            state.stats.last_error = Some(message);
            state.stats.last_failure_at = Some(Utc::now().to_rfc3339());
            state.stats.errors += 1;
            state.dirty_since = Some(
                state
                    .dirty_since
                    .map_or(dirty_since, |pending| pending.min(dirty_since)),
            );
        }
    }
    drop(state);
    shared.wake.notify_all();
    if let Some(message) = warning {
        tracing::warn!("Cannot persist reconstructible crawler observation: {message}");
    }
}

struct SnapshotBuffer(Vec<u8>);

impl Write for SnapshotBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.len() > MAX_SNAPSHOT_BYTES.saturating_sub(self.0.len()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "crawler snapshot exceeds 16 MiB",
            ));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn persist(
    path: &Path,
    projection: &Projection,
    counters: &IoCounters,
    legacy: bool,
) -> anyhow::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut writer = CountingWriter {
        file: fs::File::create(&tmp)?,
        counters,
    };
    if legacy {
        serde_json::to_writer(&mut writer, projection)?;
        writer.write_all(b"\n")?;
        writer.file.sync_all()?;
    } else {
        // Event, site and history counts are bounded; serialization never holds producer locks.
        let mut bytes = SnapshotBuffer(Vec::new());
        serde_json::to_writer(&mut bytes, projection)?;
        bytes.write_all(b"\n")?;
        writer.write_all(&bytes.0)?;
    }
    drop(writer);
    fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    include!("observer_tests.rs");
}
