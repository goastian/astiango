//! Admission and bounded incremental decoding of crawler response bodies.
//!
//! A reservation covers the wire allowance and three decoded-text allowances
//! (text, datum clone and sink copy). It survives decoding through `BodyLease`.
//! This is a body budget, not a bound on total RSS: socket buffers, parser trees
//! and the aggregate WARC buffer have their own ownership and limits.

use std::{
    ops::Deref,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

use encoding_rs::{CoderResult, Encoding, UTF_8};
use futures::StreamExt;
use reqwest::header::{CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::warc::PayloadType;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BodyLimits {
    pub wire_limit_bytes: usize,
    pub decoded_limit_bytes: usize,
}

impl Default for BodyLimits {
    fn default() -> Self {
        Self {
            wire_limit_bytes: 32 * 1024 * 1024,
            decoded_limit_bytes: 32 * 1024 * 1024,
        }
    }
}

impl BodyLimits {
    fn reservation(&self) -> Option<usize> {
        self.decoded_limit_bytes
            .checked_mul(3)?
            .checked_add(self.wire_limit_bytes)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BodyConfig {
    pub html: BodyLimits,
    pub feed: BodyLimits,
    pub sitemap: BodyLimits,
    pub robots: BodyLimits,
    pub max_inflight_bodies: usize,
    pub max_reserved_bytes: usize,
    pub connect_timeout_ms: u64,
    pub header_timeout_ms: u64,
    pub idle_timeout_ms: u64,
    pub total_timeout_ms: u64,
    pub budget_wait_timeout_ms: u64,
}

impl Default for BodyConfig {
    fn default() -> Self {
        Self {
            html: BodyLimits::default(),
            feed: BodyLimits::default(),
            sitemap: BodyLimits::default(),
            robots: BodyLimits {
                wire_limit_bytes: 512 * 1024,
                decoded_limit_bytes: 512 * 1024,
            },
            max_inflight_bodies: 8,
            max_reserved_bytes: 512 * 1024 * 1024,
            connect_timeout_ms: 10_000,
            header_timeout_ms: 30_000,
            idle_timeout_ms: 10_000,
            total_timeout_ms: 60_000,
            budget_wait_timeout_ms: 10_000,
        }
    }
}

impl BodyConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_inflight_bodies > 0,
            "body max_inflight_bodies must be positive"
        );
        anyhow::ensure!(
            self.max_reserved_bytes > 0,
            "body max_reserved_bytes must be positive"
        );
        for limits in [&self.html, &self.feed, &self.sitemap, &self.robots] {
            anyhow::ensure!(
                limits.wire_limit_bytes > 0 && limits.decoded_limit_bytes > 0,
                "body wire and decoded limits must be positive"
            );
            anyhow::ensure!(
                limits
                    .reservation()
                    .is_some_and(|n| n <= self.max_reserved_bytes),
                "each body reservation (wire + 3 * decoded) must fit max_reserved_bytes"
            );
        }
        for timeout in [
            self.connect_timeout_ms,
            self.header_timeout_ms,
            self.idle_timeout_ms,
            self.total_timeout_ms,
            self.budget_wait_timeout_ms,
        ] {
            anyhow::ensure!(
                (1..=86_400_000).contains(&timeout),
                "body timeouts must be 1..86400000 ms"
            );
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    Page,
    Html,
    Feed,
    Sitemap,
    Robots,
}

/// Absolute request lifetime, preserved when transport wraps a leased body.
#[derive(Clone, Copy, Debug)]
pub struct RequestDeadline {
    pub started: Instant,
    pub expires: Instant,
}

#[derive(Debug, thiserror::Error)]
pub enum BodyError {
    #[error("status_rejected: HTTP {0}")]
    StatusRejected(u16),
    #[error("unsupported_mime: {0}")]
    UnsupportedMime(String),
    #[error("unsupported_encoding: {0}")]
    UnsupportedEncoding(String),
    #[error("invalid_content_length")]
    InvalidContentLength,
    #[error("content_length_mismatch: declared {declared}, received {received}")]
    ContentLengthMismatch { declared: usize, received: usize },
    #[error("wire_limit_exceeded: limit {limit}")]
    WireLimitExceeded { limit: usize },
    #[error("decoded_limit_exceeded: limit {limit}")]
    DecodedLimitExceeded { limit: usize },
    #[error("connect_timeout")]
    ConnectTimeout,
    #[error("headers_timeout")]
    HeadersTimeout,
    #[error("idle_timeout")]
    IdleTimeout,
    #[error("total_timeout")]
    TotalTimeout,
    #[error("budget_timeout")]
    BudgetTimeout,
    #[error("stream_failure: {0}")]
    StreamFailure(String),
    #[error("allocation_failure")]
    AllocationFailure,
}

impl BodyError {
    pub fn reason(&self) -> &'static str {
        match self {
            Self::StatusRejected(_) => "status_rejected",
            Self::UnsupportedMime(_) => "unsupported_mime",
            Self::UnsupportedEncoding(_) => "unsupported_encoding",
            Self::InvalidContentLength => "invalid_content_length",
            Self::ContentLengthMismatch { .. } => "content_length_mismatch",
            Self::WireLimitExceeded { .. } => "wire_limit_exceeded",
            Self::DecodedLimitExceeded { .. } => "decoded_limit_exceeded",
            Self::ConnectTimeout => "connect_timeout",
            Self::HeadersTimeout => "headers_timeout",
            Self::IdleTimeout => "idle_timeout",
            Self::TotalTimeout => "total_timeout",
            Self::BudgetTimeout => "budget_timeout",
            Self::StreamFailure(_) => "stream_failure",
            Self::AllocationFailure => "allocation_failure",
        }
    }

    pub fn is_retryable(&self) -> bool {
        match self {
            Self::StatusRejected(status) => matches!(*status, 408 | 429 | 500..=599),
            Self::ConnectTimeout
            | Self::HeadersTimeout
            | Self::IdleTimeout
            | Self::TotalTimeout
            | Self::BudgetTimeout
            | Self::StreamFailure(_)
            | Self::ContentLengthMismatch { .. }
            | Self::AllocationFailure => true,
            _ => false,
        }
    }
}

macro_rules! counters {
    ($($name:ident),* $(,)?) => {
        #[derive(Clone, Debug, Default, Serialize)]
        pub struct BodyStats {
            $(pub $name: u64,)*
            pub active_bodies: u64, pub peak_active_bodies: u64,
            pub reserved_bytes: u64, pub peak_reserved_bytes: u64,
            pub max_inflight_bodies: u64, pub max_reserved_bytes: u64,
        }
        #[derive(Default)]
        struct Counters { $($name: AtomicU64,)* }
        impl Counters {
            fn snapshot(&self) -> BodyStats {
                BodyStats { $($name: self.$name.load(Ordering::Relaxed),)* ..Default::default() }
            }
        }
    }
}
counters!(
    bodies_started,
    bodies_admitted,
    bodies_completed,
    body_failures,
    rejected_status,
    rejected_mime,
    rejected_content_length,
    rejected_encoding,
    wire_limit_exceeded,
    decoded_limit_exceeded,
    stream_failures,
    allocation_failures,
    idle_timeouts,
    total_timeouts,
    budget_timeouts,
    budget_waits,
    budget_wait_ms,
    wire_bytes_received,
    decoded_bytes_produced
);

#[derive(Default)]
struct Budget {
    count: usize,
    bytes: usize,
    peak_count: usize,
    peak_bytes: usize,
}
struct Inner {
    config: BodyConfig,
    budget: Mutex<Budget>,
    changed: Notify,
    counters: Counters,
}

#[derive(Clone)]
pub struct BodyReader {
    inner: Arc<Inner>,
}

pub struct BodyLease {
    inner: Arc<Inner>,
    reservation: usize,
}
impl Drop for BodyLease {
    fn drop(&mut self) {
        let mut budget = self.inner.budget.lock().unwrap();
        budget.count -= 1;
        budget.bytes -= self.reservation;
        drop(budget);
        self.inner.changed.notify_waiters();
    }
}

pub struct AdmittedBody {
    text: String,
    lease: BodyLease,
    kind: BodyKind,
    payload_type: Option<PayloadType>,
}
impl std::fmt::Debug for AdmittedBody {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdmittedBody")
            .field("len", &self.text.len())
            .field("kind", &self.kind)
            .field("payload_type", &self.payload_type)
            .finish()
    }
}
impl AdmittedBody {
    pub fn as_str(&self) -> &str {
        &self.text
    }
    pub fn kind(&self) -> BodyKind {
        self.kind
    }
    pub fn payload_type(&self) -> Option<PayloadType> {
        self.payload_type
    }
    pub fn into_parts(self) -> (String, BodyLease) {
        (self.text, self.lease)
    }
}
impl Deref for AdmittedBody {
    type Target = str;
    fn deref(&self) -> &str {
        &self.text
    }
}

struct Admission {
    kind: BodyKind,
    payload_type: Option<PayloadType>,
    encoding: &'static Encoding,
    declared: Option<usize>,
    limits: BodyLimits,
}

impl BodyReader {
    pub fn new(config: BodyConfig) -> anyhow::Result<Self> {
        config.validate()?;
        Ok(Self {
            inner: Arc::new(Inner {
                config,
                budget: Mutex::new(Budget::default()),
                changed: Notify::new(),
                counters: Counters::default(),
            }),
        })
    }

    pub fn stats(&self) -> BodyStats {
        let mut stats = self.inner.counters.snapshot();
        let budget = self.inner.budget.lock().unwrap();
        stats.active_bodies = budget.count as u64;
        stats.peak_active_bodies = budget.peak_count as u64;
        stats.reserved_bytes = budget.bytes as u64;
        stats.peak_reserved_bytes = budget.peak_bytes as u64;
        stats.max_inflight_bodies = self.inner.config.max_inflight_bodies as u64;
        stats.max_reserved_bytes = self.inner.config.max_reserved_bytes as u64;
        stats
    }

    pub async fn read(
        &self,
        response: reqwest::Response,
        kind: BodyKind,
    ) -> Result<AdmittedBody, BodyError> {
        self.read_with_admission(response, kind, false).await
    }

    /// Compatibility for the public decoder helper, whose caller owns admission.
    /// This deliberately bypasses only status/MIME admission, not resource limits.
    pub(crate) async fn read_legacy(
        &self,
        response: reqwest::Response,
    ) -> Result<AdmittedBody, BodyError> {
        self.read_with_admission(response, BodyKind::Html, true)
            .await
    }

    async fn read_with_admission(
        &self,
        response: reqwest::Response,
        kind: BodyKind,
        legacy: bool,
    ) -> Result<AdmittedBody, BodyError> {
        self.inner
            .counters
            .bodies_started
            .fetch_add(1, Ordering::Relaxed);
        let result = self.read_inner(response, kind, legacy).await;
        if let Err(error) = &result {
            self.record_error(error);
        }
        result
    }

    fn record_error(&self, error: &BodyError) {
        let c = &self.inner.counters;
        c.body_failures.fetch_add(1, Ordering::Relaxed);
        let counter = match error {
            BodyError::StatusRejected(_) => &c.rejected_status,
            BodyError::UnsupportedMime(_) => &c.rejected_mime,
            BodyError::UnsupportedEncoding(_) => &c.rejected_encoding,
            BodyError::InvalidContentLength | BodyError::ContentLengthMismatch { .. } => {
                &c.rejected_content_length
            }
            BodyError::WireLimitExceeded { .. } => &c.wire_limit_exceeded,
            BodyError::DecodedLimitExceeded { .. } => &c.decoded_limit_exceeded,
            BodyError::StreamFailure(_) => &c.stream_failures,
            BodyError::IdleTimeout => &c.idle_timeouts,
            BodyError::TotalTimeout => &c.total_timeouts,
            BodyError::BudgetTimeout => &c.budget_timeouts,
            BodyError::AllocationFailure => &c.allocation_failures,
            BodyError::ConnectTimeout | BodyError::HeadersTimeout => return,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    fn admit(
        &self,
        response: &reqwest::Response,
        requested: BodyKind,
        legacy: bool,
    ) -> Result<Admission, BodyError> {
        if !legacy && response.status() != reqwest::StatusCode::OK {
            return Err(BodyError::StatusRejected(response.status().as_u16()));
        }
        for value in response.headers().get_all(CONTENT_ENCODING) {
            let value = value
                .to_str()
                .map_err(|_| BodyError::UnsupportedEncoding("invalid header".into()))?;
            if value
                .split(',')
                .any(|part| !part.trim().eq_ignore_ascii_case("identity"))
            {
                return Err(BodyError::UnsupportedEncoding(value.to_owned()));
            }
        }
        let content_type = response.headers().get(CONTENT_TYPE);
        let mime = content_type
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<mime::Mime>().ok());
        if !legacy && content_type.is_some() && mime.is_none() {
            return Err(BodyError::UnsupportedMime("invalid header".into()));
        }
        let essence = mime.as_ref().map(|mime| mime.essence_str());
        let payload_type = match essence {
            Some("text/html" | "application/xhtml+xml") => Some(PayloadType::Html),
            Some("application/rss+xml") => Some(PayloadType::Rss),
            Some("application/atom+xml") => Some(PayloadType::Atom),
            _ => None,
        };
        let kind = if requested == BodyKind::Page {
            match payload_type {
                Some(PayloadType::Html) => BodyKind::Html,
                Some(PayloadType::Rss | PayloadType::Atom) => BodyKind::Feed,
                _ => BodyKind::Page,
            }
        } else {
            requested
        };
        let accepted = match kind {
            BodyKind::Page => false,
            BodyKind::Html => payload_type == Some(PayloadType::Html),
            BodyKind::Feed => matches!(
                essence,
                None | Some(
                    "application/rss+xml"
                        | "application/atom+xml"
                        | "application/xml"
                        | "text/xml"
                        | "text/plain"
                )
            ),
            BodyKind::Sitemap => matches!(
                essence,
                None | Some("application/xml" | "text/xml" | "text/plain")
            ),
            // HTML is kept for compatibility with historic robots endpoints.
            BodyKind::Robots => matches!(essence, None | Some("text/plain" | "text/html")),
        };
        if !legacy && !accepted {
            return Err(BodyError::UnsupportedMime(
                essence.unwrap_or("missing").to_owned(),
            ));
        }
        let encoding = mime
            .as_ref()
            .and_then(|mime| mime.get_param(mime::CHARSET))
            .and_then(|label| Encoding::for_label(label.as_str().as_bytes()))
            .unwrap_or(UTF_8);
        let limits = match kind {
            BodyKind::Html | BodyKind::Page => &self.inner.config.html,
            BodyKind::Feed => &self.inner.config.feed,
            BodyKind::Sitemap => &self.inner.config.sitemap,
            BodyKind::Robots => &self.inner.config.robots,
        }
        .clone();
        let mut declared = None;
        for value in response.headers().get_all(CONTENT_LENGTH) {
            for part in value
                .to_str()
                .map_err(|_| BodyError::InvalidContentLength)?
                .split(',')
            {
                let part = part.trim();
                if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(BodyError::InvalidContentLength);
                }
                let length = part
                    .parse::<usize>()
                    .map_err(|_| BodyError::InvalidContentLength)?;
                if declared.is_some_and(|old| old != length) {
                    return Err(BodyError::InvalidContentLength);
                }
                if length > limits.wire_limit_bytes {
                    self.inner
                        .counters
                        .rejected_content_length
                        .fetch_add(1, Ordering::Relaxed);
                    return Err(BodyError::WireLimitExceeded {
                        limit: limits.wire_limit_bytes,
                    });
                }
                declared = Some(length);
            }
        }
        Ok(Admission {
            kind,
            payload_type,
            encoding,
            declared,
            limits,
        })
    }

    async fn reserve(&self, reservation: usize, total: Instant) -> Result<BodyLease, BodyError> {
        let started = Instant::now();
        let deadline =
            total.min(started + Duration::from_millis(self.inner.config.budget_wait_timeout_ms));
        let mut wait = BudgetWait {
            reader: self,
            started,
            waited: false,
        };
        loop {
            let notified = self.inner.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if Instant::now() >= deadline {
                return Err(if Instant::now() >= total {
                    BodyError::TotalTimeout
                } else {
                    BodyError::BudgetTimeout
                });
            }
            {
                let mut budget = self.inner.budget.lock().unwrap();
                if budget.count < self.inner.config.max_inflight_bodies
                    && reservation <= self.inner.config.max_reserved_bytes - budget.bytes
                {
                    budget.count += 1;
                    budget.bytes += reservation;
                    budget.peak_count = budget.peak_count.max(budget.count);
                    budget.peak_bytes = budget.peak_bytes.max(budget.bytes);
                    return Ok(BodyLease {
                        inner: self.inner.clone(),
                        reservation,
                    });
                }
            }
            if !wait.waited {
                wait.waited = true;
                self.inner
                    .counters
                    .budget_waits
                    .fetch_add(1, Ordering::Relaxed);
            }
            let _ = tokio::time::timeout_at(deadline.into(), notified).await;
        }
    }

    async fn read_inner(
        &self,
        response: reqwest::Response,
        kind: BodyKind,
        legacy: bool,
    ) -> Result<AdmittedBody, BodyError> {
        let admission = self.admit(&response, kind, legacy)?;
        let now = Instant::now();
        let total_duration = Duration::from_millis(self.inner.config.total_timeout_ms);
        let deadline = response
            .extensions()
            .get::<RequestDeadline>()
            .map(|request| request.expires.min(request.started + total_duration))
            .unwrap_or(now + total_duration);
        let lease = self
            .reserve(admission.limits.reservation().unwrap(), deadline)
            .await?;
        self.inner
            .counters
            .bodies_admitted
            .fetch_add(1, Ordering::Relaxed);
        let mut text = String::new();
        text.try_reserve_exact(admission.limits.decoded_limit_bytes)
            .map_err(|_| BodyError::AllocationFailure)?;
        let mut decoder = admission.encoding.new_decoder();
        let mut stream = response.bytes_stream();
        let mut received = 0usize;
        let mut last_progress = Instant::now();
        loop {
            let idle = last_progress + Duration::from_millis(self.inner.config.idle_timeout_ms);
            if Instant::now() >= deadline.min(idle) {
                return Err(if Instant::now() >= deadline {
                    BodyError::TotalTimeout
                } else {
                    BodyError::IdleTimeout
                });
            }
            let next = tokio::time::timeout_at(deadline.min(idle).into(), stream.next())
                .await
                .map_err(|_| {
                    if Instant::now() >= deadline {
                        BodyError::TotalTimeout
                    } else {
                        BodyError::IdleTimeout
                    }
                })?;
            if Instant::now() >= deadline {
                return Err(BodyError::TotalTimeout);
            }
            match next {
                Some(Ok(chunk)) => {
                    self.inner
                        .counters
                        .wire_bytes_received
                        .fetch_add(chunk.len() as u64, Ordering::Relaxed);
                    if chunk.len() > admission.limits.wire_limit_bytes - received {
                        return Err(BodyError::WireLimitExceeded {
                            limit: admission.limits.wire_limit_bytes,
                        });
                    }
                    received += chunk.len();
                    if !chunk.is_empty() {
                        last_progress = Instant::now();
                    }
                    self.decode(
                        &mut decoder,
                        &chunk,
                        false,
                        &mut text,
                        admission.limits.decoded_limit_bytes,
                        deadline,
                    )?;
                }
                Some(Err(error)) => {
                    return Err(if error.is_timeout() && Instant::now() >= deadline {
                        BodyError::TotalTimeout
                    } else {
                        BodyError::StreamFailure(error.to_string())
                    });
                }
                None => break,
            }
        }
        if let Some(declared) = admission.declared {
            if declared != received {
                return Err(BodyError::ContentLengthMismatch { declared, received });
            }
        }
        self.decode(
            &mut decoder,
            &[],
            true,
            &mut text,
            admission.limits.decoded_limit_bytes,
            deadline,
        )?;
        self.inner
            .counters
            .bodies_completed
            .fetch_add(1, Ordering::Relaxed);
        Ok(AdmittedBody {
            text,
            lease,
            kind: admission.kind,
            payload_type: admission.payload_type,
        })
    }

    fn decode(
        &self,
        decoder: &mut encoding_rs::Decoder,
        mut input: &[u8],
        last: bool,
        text: &mut String,
        limit: usize,
        deadline: Instant,
    ) -> Result<(), BodyError> {
        let mut output = [0u8; 8192];
        loop {
            if Instant::now() >= deadline {
                return Err(BodyError::TotalTimeout);
            }
            let (result, read, written, _) = decoder.decode_to_utf8(input, &mut output, last);
            if written > limit - text.len() {
                return Err(BodyError::DecodedLimitExceeded { limit });
            }
            // encoding_rs guarantees complete, valid UTF-8 in each output buffer.
            text.push_str(
                std::str::from_utf8(&output[..written]).expect("decoder emitted valid UTF-8"),
            );
            self.inner
                .counters
                .decoded_bytes_produced
                .fetch_add(written as u64, Ordering::Relaxed);
            input = &input[read..];
            if result == CoderResult::InputEmpty {
                return Ok(());
            }
        }
    }
}

struct BudgetWait<'a> {
    reader: &'a BodyReader,
    started: Instant,
    waited: bool,
}
impl Drop for BudgetWait<'_> {
    fn drop(&mut self) {
        if self.waited {
            self.reader.inner.counters.budget_wait_ms.fetch_add(
                self.started.elapsed().as_millis().min(u64::MAX as u128) as u64,
                Ordering::Relaxed,
            );
        }
    }
}

#[cfg(test)]
#[path = "body_tests.rs"]
mod tests;
