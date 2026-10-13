use std::{
    collections::VecDeque,
    panic::{catch_unwind, AssertUnwindSafe},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex, Weak,
    },
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, Notify};

use super::{body::BodyLease, extraction::CrawlExtraction, CrawlDatum};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct StageConfig {
    pub workers: usize,
    pub max_items: usize,
    pub max_bytes: usize,
}
impl Default for StageConfig {
    fn default() -> Self {
        Self {
            workers: 2,
            max_items: 4,
            max_bytes: 128 * 1024 * 1024,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct PipelineConfig {
    pub max_inflight_documents: usize,
    pub max_inflight_bytes: usize,
    pub parse: StageConfig,
    pub network_setup: StageConfig,
    pub storage: StageConfig,
    pub control: StageConfig,
    pub queue_wait_timeout_ms: u64,
    pub runtime_sample_interval_ms: u64,
    pub batch_segment_bytes: usize,
    pub batch_segment_items: usize,
}
impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            max_inflight_documents: 8,
            max_inflight_bytes: 256 * 1024 * 1024,
            parse: StageConfig::default(),
            network_setup: StageConfig {
                workers: 1,
                max_items: 2,
                max_bytes: 1024 * 1024,
            },
            storage: StageConfig {
                workers: 1,
                ..StageConfig::default()
            },
            control: StageConfig {
                workers: 1,
                max_items: 2,
                max_bytes: 16 * 1024 * 1024,
            },
            queue_wait_timeout_ms: 30_000,
            runtime_sample_interval_ms: 25,
            batch_segment_bytes: 64 * 1024 * 1024,
            batch_segment_items: 512,
        }
    }
}
impl PipelineConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=1024).contains(&self.max_inflight_documents),
            "pipeline max_inflight_documents must be 1..1024"
        );
        anyhow::ensure!(
            self.max_inflight_bytes > 0 && self.batch_segment_bytes > 0,
            "pipeline byte budgets must be positive"
        );
        anyhow::ensure!(
            (1..=100_000).contains(&self.batch_segment_items),
            "pipeline batch_segment_items must be 1..100000"
        );
        for stage in [
            &self.parse,
            &self.network_setup,
            &self.storage,
            &self.control,
        ] {
            anyhow::ensure!(
                (1..=32).contains(&stage.workers),
                "pipeline stage workers must be 1..32"
            );
            anyhow::ensure!(
                stage.max_items >= stage.workers && stage.max_items <= 1024 && stage.max_bytes > 0,
                "pipeline stage capacity must include workers and have positive bytes"
            );
        }
        anyhow::ensure!(
            (1..=86_400_000).contains(&self.queue_wait_timeout_ms),
            "pipeline queue timeout must be 1..86400000 ms"
        );
        anyhow::ensure!(
            (10..=1000).contains(&self.runtime_sample_interval_ms),
            "pipeline runtime sample interval must be 10..1000 ms"
        );
        Ok(())
    }
    pub fn validate_body(&self, body: &super::body::BodyConfig) -> anyhow::Result<()> {
        self.validate()?;
        let page = body
            .html
            .decoded_limit_bytes
            .max(body.feed.decoded_limit_bytes);
        let parsed = page
            .max(body.sitemap.decoded_limit_bytes)
            .max(body.robots.decoded_limit_bytes);
        anyhow::ensure!(
            page <= self.max_inflight_bytes
                && parsed <= self.parse.max_bytes
                && page <= self.storage.max_bytes,
            "a decoded body must fit pipeline admission, parse and storage byte budgets"
        );
        anyhow::ensure!(
            page <= self.batch_segment_bytes && self.batch_segment_bytes <= self.storage.max_bytes,
            "pipeline batch_segment_bytes must fit a page and the storage stage"
        );
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("pipeline_closed")]
    Closed,
    #[error("pipeline_queue_timeout")]
    QueueTimeout,
    #[error("pipeline_input_too_large: {bytes} bytes exceed {limit}")]
    TooLarge { bytes: usize, limit: usize },
    #[error("pipeline_worker_failed")]
    WorkerFailed,
}

#[derive(Default, Clone, Serialize)]
pub struct StageStats {
    pub workers: usize,
    pub max_items: usize,
    pub max_bytes: usize,
    pub items: usize,
    pub bytes: usize,
    pub running: usize,
    pub queued: usize,
    pub peak_items: usize,
    pub peak_bytes: usize,
    pub peak_running: usize,
    pub peak_queued: usize,
    pub submitted: u64,
    pub completed: u64,
    pub cancelled: u64,
    pub failed: u64,
    pub admission_waits: u64,
    pub admission_wait_us: u64,
    pub queue_wait_us: u64,
    pub work_us: u64,
    pub queue_timeouts: u64,
    pub input_rejections: u64,
}
#[derive(Default)]
struct State {
    items: usize,
    bytes: usize,
    running: usize,
    peak_items: usize,
    peak_bytes: usize,
    peak_running: usize,
    peak_queued: usize,
}
struct Budget {
    config: StageConfig,
    state: Mutex<State>,
    changed: Notify,
    closed: AtomicBool,
    submitted: AtomicU64,
    completed: AtomicU64,
    cancelled: AtomicU64,
    failed: AtomicU64,
    admission_waits: AtomicU64,
    admission_wait_us: AtomicU64,
    queue_wait_us: AtomicU64,
    work_us: AtomicU64,
    queue_timeouts: AtomicU64,
    input_rejections: AtomicU64,
}
impl Budget {
    fn new(config: StageConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            state: Mutex::new(State::default()),
            changed: Notify::new(),
            closed: AtomicBool::new(false),
            submitted: AtomicU64::new(0),
            completed: AtomicU64::new(0),
            cancelled: AtomicU64::new(0),
            failed: AtomicU64::new(0),
            admission_waits: AtomicU64::new(0),
            admission_wait_us: AtomicU64::new(0),
            queue_wait_us: AtomicU64::new(0),
            work_us: AtomicU64::new(0),
            queue_timeouts: AtomicU64::new(0),
            input_rejections: AtomicU64::new(0),
        })
    }
    async fn acquire(
        self: &Arc<Self>,
        bytes: usize,
        timeout: Duration,
    ) -> Result<Lease, PipelineError> {
        if bytes > self.config.max_bytes {
            self.input_rejections.fetch_add(1, Ordering::Relaxed);
            return Err(PipelineError::TooLarge {
                bytes,
                limit: self.config.max_bytes,
            });
        }
        let started = Instant::now();
        let deadline = tokio::time::Instant::now() + timeout;
        let mut waited = false;
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            {
                let mut state = self.state.lock().unwrap();
                if self.closed.load(Ordering::Acquire) {
                    return Err(PipelineError::Closed);
                }
                if state.items < self.config.max_items
                    && bytes <= self.config.max_bytes - state.bytes
                {
                    state.items += 1;
                    state.bytes += bytes;
                    state.peak_items = state.peak_items.max(state.items);
                    state.peak_bytes = state.peak_bytes.max(state.bytes);
                    state.peak_queued = state.peak_queued.max(state.items - state.running);
                    if waited {
                        self.admission_wait_us
                            .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
                    }
                    return Ok(Lease {
                        budget: self.clone(),
                        bytes,
                        started: None,
                        count_release: false,
                        admitted: Instant::now(),
                    });
                }
            }
            if !waited {
                self.admission_waits.fetch_add(1, Ordering::Relaxed);
                waited = true;
            }
            if tokio::time::timeout_at(deadline, &mut changed)
                .await
                .is_err()
            {
                self.queue_timeouts.fetch_add(1, Ordering::Relaxed);
                return Err(PipelineError::QueueTimeout);
            }
        }
    }
    async fn drain(&self) {
        loop {
            let changed = self.changed.notified();
            tokio::pin!(changed);
            changed.as_mut().enable();
            if self.state.lock().unwrap().items == 0 {
                return;
            }
            changed.await;
        }
    }
    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.changed.notify_waiters();
    }
    fn stats(&self) -> StageStats {
        let state = self.state.lock().unwrap();
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        StageStats {
            workers: self.config.workers,
            max_items: self.config.max_items,
            max_bytes: self.config.max_bytes,
            items: state.items,
            bytes: state.bytes,
            running: state.running,
            queued: state.items - state.running,
            peak_items: state.peak_items,
            peak_bytes: state.peak_bytes,
            peak_running: state.peak_running,
            peak_queued: state.peak_queued,
            submitted: load(&self.submitted),
            completed: load(&self.completed),
            cancelled: load(&self.cancelled),
            failed: load(&self.failed),
            admission_waits: load(&self.admission_waits),
            admission_wait_us: load(&self.admission_wait_us),
            queue_wait_us: load(&self.queue_wait_us),
            work_us: load(&self.work_us),
            queue_timeouts: load(&self.queue_timeouts),
            input_rejections: load(&self.input_rejections),
        }
    }
}
pub struct Lease {
    budget: Arc<Budget>,
    bytes: usize,
    admitted: Instant,
    started: Option<Instant>,
    count_release: bool,
}
impl Lease {
    pub(crate) fn start(&mut self) {
        let mut state = self.budget.state.lock().unwrap();
        state.running += 1;
        state.peak_running = state.peak_running.max(state.running);
        self.budget.queue_wait_us.fetch_add(
            self.admitted.elapsed().as_micros() as u64,
            Ordering::Relaxed,
        );
        self.started = Some(Instant::now());
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let mut state = self.budget.state.lock().unwrap();
        state.items -= 1;
        state.bytes -= self.bytes;
        if let Some(started) = self.started {
            state.running -= 1;
            self.budget
                .work_us
                .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
        }
        drop(state);
        if self.count_release {
            self.budget.completed.fetch_add(1, Ordering::Relaxed);
        }
        self.budget.changed.notify_waiters();
    }
}

type Work = Box<dyn FnOnce() + Send + 'static>;
#[derive(Clone)]
pub struct BlockingStage {
    budget: Arc<Budget>,
    sender: Arc<Mutex<Option<mpsc::Sender<Work>>>>,
    timeout: Duration,
    #[cfg(any(test, feature = "crawler-bench"))]
    inline: bool,
    #[cfg(any(test, feature = "crawler-bench"))]
    delay_ms: Arc<AtomicU64>,
}
impl BlockingStage {
    pub(super) fn new(config: StageConfig, timeout: Duration) -> Self {
        Self {
            budget: Budget::new(config),
            sender: Arc::new(Mutex::new(None)),
            timeout,
            #[cfg(any(test, feature = "crawler-bench"))]
            inline: false,
            #[cfg(any(test, feature = "crawler-bench"))]
            delay_ms: Arc::new(AtomicU64::new(0)),
        }
    }
    fn sender(&self) -> Result<mpsc::Sender<Work>, PipelineError> {
        let mut sender = self.sender.lock().unwrap();
        if self.budget.closed.load(Ordering::Acquire) {
            return Err(PipelineError::Closed);
        }
        Ok(sender
            .get_or_insert_with(|| {
                let (tx, rx) = mpsc::channel::<Work>(self.budget.config.max_items);
                let rx = Arc::new(Mutex::new(rx));
                for _ in 0..self.budget.config.workers {
                    let rx = rx.clone();
                    tokio::task::spawn_blocking(move || loop {
                        let work = rx.lock().unwrap().blocking_recv();
                        let Some(work) = work else { break };
                        work();
                    });
                }
                tx
            })
            .clone())
    }
    pub async fn run<T: Send + 'static>(
        &self,
        bytes: usize,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, PipelineError> {
        self.execute(bytes, work, true).await
    }
    pub async fn run_durable<T: Send + 'static>(
        &self,
        bytes: usize,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, PipelineError> {
        self.execute(bytes, work, false).await
    }
    async fn execute<T: Send + 'static>(
        &self,
        bytes: usize,
        work: impl FnOnce() -> T + Send + 'static,
        skip_cancelled: bool,
    ) -> Result<T, PipelineError> {
        let mut lease = self.budget.acquire(bytes, self.timeout).await?;
        let budget = self.budget.clone();
        let (reply, result) = oneshot::channel();
        #[cfg(any(test, feature = "crawler-bench"))]
        let delay = self.delay_ms.load(Ordering::Relaxed);
        let job: Work = Box::new(move || {
            if skip_cancelled && reply.is_closed() {
                budget.cancelled.fetch_add(1, Ordering::Relaxed);
                return;
            }
            lease.start();
            #[cfg(any(test, feature = "crawler-bench"))]
            std::thread::sleep(Duration::from_millis(delay));
            let output =
                catch_unwind(AssertUnwindSafe(work)).map_err(|_| PipelineError::WorkerFailed);
            if output.is_ok() {
                budget.completed.fetch_add(1, Ordering::Relaxed);
            } else {
                budget.failed.fetch_add(1, Ordering::Relaxed);
            }
            drop(lease);
            let _ = reply.send(output);
        });
        self.budget.submitted.fetch_add(1, Ordering::Relaxed);
        #[cfg(any(test, feature = "crawler-bench"))]
        if self.inline {
            job();
            return result.await.map_err(|_| PipelineError::WorkerFailed)?;
        }
        let sender = self.sender()?;
        sender.send(job).await.map_err(|_| PipelineError::Closed)?;
        drop(sender);
        result.await.map_err(|_| PipelineError::WorkerFailed)?
    }
    pub fn stats(&self) -> StageStats {
        self.budget.stats()
    }
    pub async fn drain(&self) {
        self.budget.drain().await;
    }
    fn close(&self) {
        self.budget.close();
        self.sender.lock().unwrap().take();
    }
}

struct Lag {
    samples: Mutex<VecDeque<u64>>,
    total: AtomicU64,
    max_us: AtomicU64,
    started: AtomicBool,
    stopped: AtomicBool,
}
struct Inner {
    config: PipelineConfig,
    admission: Arc<Budget>,
    lag: Arc<Lag>,
}
#[derive(Clone)]
pub struct Pipeline {
    inner: Arc<Inner>,
    pub parse: BlockingStage,
    pub network_setup: BlockingStage,
    pub storage: BlockingStage,
    pub control: BlockingStage,
    pub(crate) batch_queue: ByteQueue,
}
#[derive(Clone)]
pub(crate) struct ByteQueue {
    budget: Arc<Budget>,
    timeout: Duration,
}
impl ByteQueue {
    pub(crate) async fn acquire(&self, bytes: usize) -> Result<Lease, PipelineError> {
        let mut lease = self.budget.acquire(bytes, self.timeout).await?;
        self.budget.submitted.fetch_add(1, Ordering::Relaxed);
        lease.count_release = true;
        Ok(lease)
    }
}
#[derive(Clone, Serialize)]
pub struct PipelineStats {
    pub admission: StageStats,
    pub parse: StageStats,
    pub network_setup: StageStats,
    pub storage: StageStats,
    pub control: StageStats,
    pub accepting: bool,
    pub runtime_lag_samples: u64,
    pub runtime_lag_window_samples: usize,
    pub runtime_lag_p99_us: u64,
    pub runtime_lag_max_us: u64,
    pub runtime_sample_interval_ms: u64,
    pub batch_segment_bytes: usize,
    pub batch_segment_items: usize,
    pub batch_queue: StageStats,
}
impl Pipeline {
    pub(crate) async fn prepare_page(
        &self,
        mut datum: CrawlDatum,
        body_lease: BodyLease,
        admission: Lease,
        extractor: super::extraction::Extractor,
        domain: super::Domain,
        visited: Arc<super::extraction::VisitedUrls>,
        requested_url: url::Url,
    ) -> Result<PreparedPage, PipelineError> {
        self.parse
            .run(datum.memory_bytes(), move || {
                let mut extraction = extractor
                    .parse_capture(
                        &datum.body,
                        &datum.url,
                        super::extraction::ExtractionContext {
                            domain: &domain,
                            crawled_urls: &visited,
                            requested_url: &requested_url,
                        },
                        datum
                            .capture
                            .as_ref()
                            .map_or(crate::capture::DEFAULT_AGENT, |capture| {
                                capture.metadata.user_agent.as_str()
                            }),
                    )
                    .ok();
                if let Some(capture) = datum.capture.as_mut() {
                    capture.metadata.payload_sha256 = crate::capture::sha256(&capture.payload);
                    if let Some(value) = extraction.as_mut() {
                        for directive in &value.robots_directives {
                            capture
                                .metadata
                                .robots
                                .add(directive.clone(), &capture.metadata.user_agent);
                        }
                        capture.metadata.robots.directives_seen += value
                            .robots_directives_seen
                            .saturating_sub(value.robots_directives.len() as u64);
                        capture.metadata.robots.directives_truncated |=
                            value.robots_directives_truncated;
                        capture.metadata.robots.no_index |= value.no_index;
                        capture.metadata.robots.no_follow |= value.no_follow;
                        value.no_index = capture.metadata.robots.no_index;
                        value.no_follow = capture.metadata.robots.no_follow;
                        if value.no_follow {
                            value.discovery_urls.clear();
                            value.internal_urls.clear();
                        }
                        capture.metadata.discovery_urls = value.discovery_urls.clone();
                        capture.metadata.internal_urls = value.internal_urls.clone();
                    }
                }
                PreparedPage {
                    datum,
                    extraction,
                    body_lease: Some(body_lease),
                    admission: Some(admission),
                }
            })
            .await
    }
    pub fn new(mut config: PipelineConfig) -> anyhow::Result<Self> {
        let cpus = super::resources::effective_cpus();
        for stage in [&mut config.parse, &mut config.storage, &mut config.control, &mut config.network_setup] {
            stage.workers = stage.workers.min(cpus);
        }
        config.validate()?;
        let timeout = Duration::from_millis(config.queue_wait_timeout_ms);
        Ok(Self {
            parse: BlockingStage::new(config.parse.clone(), timeout),
            network_setup: BlockingStage::new(config.network_setup.clone(), timeout),
            storage: BlockingStage::new(config.storage.clone(), timeout),
            control: BlockingStage::new(config.control.clone(), timeout),
            batch_queue: ByteQueue {
                budget: Budget::new(StageConfig {
                    workers: 1,
                    ..config.storage.clone()
                }),
                timeout,
            },
            inner: Arc::new(Inner {
                admission: Budget::new(StageConfig {
                    workers: 0,
                    max_items: config.max_inflight_documents,
                    max_bytes: config.max_inflight_bytes,
                }),
                config,
                lag: Arc::new(Lag {
                    samples: Mutex::new(VecDeque::with_capacity(1024)),
                    total: AtomicU64::new(0),
                    max_us: AtomicU64::new(0),
                    started: AtomicBool::new(false),
                    stopped: AtomicBool::new(false),
                }),
            }),
        })
    }
    pub fn start_monitor(&self) {
        if self.inner.lag.started.swap(true, Ordering::AcqRel) {
            return;
        }
        let inner: Weak<Inner> = Arc::downgrade(&self.inner);
        let interval = Duration::from_millis(self.inner.config.runtime_sample_interval_ms);
        tokio::spawn(async move {
            let mut timer =
                tokio::time::interval_at(tokio::time::Instant::now() + interval, interval);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                let expected = timer.tick().await;
                let Some(inner) = inner.upgrade() else { break };
                if inner.lag.stopped.load(Ordering::Acquire) {
                    break;
                }
                let lag = tokio::time::Instant::now()
                    .saturating_duration_since(expected)
                    .as_micros() as u64;
                inner.lag.total.fetch_add(1, Ordering::Relaxed);
                inner.lag.max_us.fetch_max(lag, Ordering::Relaxed);
                let mut samples = inner.lag.samples.lock().unwrap();
                if samples.len() == 1024 {
                    samples.pop_front();
                }
                samples.push_back(lag);
            }
        });
    }
    pub async fn admit(&self, bytes: usize) -> Result<Lease, PipelineError> {
        self.start_monitor();
        let mut lease = self
            .inner
            .admission
            .acquire(
                bytes,
                Duration::from_millis(self.inner.config.queue_wait_timeout_ms),
            )
            .await?;
        self.inner
            .admission
            .submitted
            .fetch_add(1, Ordering::Relaxed);
        lease.count_release = true;
        Ok(lease)
    }
    pub fn stop_admission(&self) {
        self.inner.admission.close();
    }
    pub async fn drain(&self) {
        self.inner.admission.drain().await;
        self.parse.drain().await;
        self.network_setup.drain().await;
        self.storage.drain().await;
        self.control.drain().await;
        self.batch_queue.budget.drain().await;
    }
    pub async fn shutdown(&self) {
        self.stop_admission();
        self.drain().await;
        self.parse.close();
        self.network_setup.close();
        self.storage.close();
        self.control.close();
        self.batch_queue.budget.close();
        self.inner.lag.stopped.store(true, Ordering::Release);
    }
    pub fn stats(&self) -> PipelineStats {
        let mut samples: Vec<_> = self
            .inner
            .lag
            .samples
            .lock()
            .unwrap()
            .iter()
            .copied()
            .collect();
        samples.sort_unstable();
        let p99 = if samples.is_empty() {
            0
        } else {
            samples[(samples.len() * 99).div_ceil(100).saturating_sub(1)]
        };
        PipelineStats {
            admission: self.inner.admission.stats(),
            parse: self.parse.stats(),
            network_setup: self.network_setup.stats(),
            storage: self.storage.stats(),
            control: self.control.stats(),
            accepting: !self.inner.admission.closed.load(Ordering::Acquire),
            runtime_lag_samples: self.inner.lag.total.load(Ordering::Relaxed),
            runtime_lag_window_samples: samples.len(),
            runtime_lag_p99_us: p99,
            runtime_lag_max_us: self.inner.lag.max_us.load(Ordering::Relaxed),
            runtime_sample_interval_ms: self.inner.config.runtime_sample_interval_ms,
            batch_segment_bytes: self.inner.config.batch_segment_bytes,
            batch_segment_items: self.inner.config.batch_segment_items,
            batch_queue: self.batch_queue.budget.stats(),
        }
    }
    #[cfg(any(test, feature = "crawler-bench"))]
    pub fn fixture_mode(&mut self, inline: bool, parse_delay_ms: u64, storage_delay_ms: u64) {
        self.parse.inline = inline;
        self.network_setup.inline = inline;
        self.storage.inline = inline;
        self.control.inline = inline;
        self.parse.delay_ms.store(parse_delay_ms, Ordering::Relaxed);
        self.storage
            .delay_ms
            .store(storage_delay_ms, Ordering::Relaxed);
    }
}

pub struct PreparedPage {
    pub datum: CrawlDatum,
    pub extraction: Option<CrawlExtraction>,
    pub(crate) body_lease: Option<BodyLease>,
    pub(crate) admission: Option<Lease>,
}
impl PreparedPage {
    pub(crate) fn unleased(datum: CrawlDatum, extraction: Option<CrawlExtraction>) -> Self {
        Self {
            datum,
            extraction,
            body_lease: None,
            admission: None,
        }
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
