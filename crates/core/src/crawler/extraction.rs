//! C06: one DOM, two bounded projections, no indexer text extraction.
//! This reconstructible result is not a capture or a new WARC/RPC schema (C09).
use super::{Domain, HashMap, MAX_OUTGOING_URLS_PER_PAGE, MAX_URL_LEN_BYTES};
use crate::webpage::{url_ext::UrlExt, Html};
use serde::{Deserialize, Serialize};
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
    time::Instant,
};
use url::Url;

pub(super) const IGNORED_EXTENSIONS: [&str; 27] = [
    ".pdf", ".jpg", ".zip", ".png", ".css", ".js", ".json", ".jsonp", ".woff2", ".woff", ".ttf",
    ".svg", ".gif", ".jpeg", ".ico", ".mp4", ".mp3", ".avi", ".mov", ".mpeg", ".webm", ".wav",
    ".flac", ".aac", ".ogg", ".m4a", ".m4v",
];

pub type VisitedUrls = hashbrown::HashSet<Url>;

pub struct ExtractionContext<'a> {
    pub domain: &'a Domain,
    pub crawled_urls: &'a VisitedUrls,
    pub requested_url: &'a Url,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CrawlExtraction {
    pub schema_version: u32,
    pub url: Url,
    pub base_url: Url,
    pub canonical_url: Option<Url>,
    pub no_index: bool,
    pub no_follow: bool,
    /// Preserve the historical local prefix, including duplicate destinations.
    pub discovery_urls: Vec<Url>,
    /// Normalize, filter and deduplicate before the independent internal quota.
    pub internal_urls: Vec<Url>,
    pub anchors_visited: u64,
    pub discovery_limit_reached: bool,
    pub internal_limit_reached: bool,
    pub traversal_stopped: bool,
}

impl CrawlExtraction {
    pub fn parse(body: &str, url: &Url, context: ExtractionContext<'_>) -> anyhow::Result<Self> {
        let html = Html::parse_without_text(body, url.as_str())?;
        let mut result = Self::metadata(&html);
        // This set contains only retained internal URLs, never all page anchors.
        let mut seen: HashMap<Url, ()> = HashMap::default();
        result.anchors_visited = html.visit_anchor_destinations(|destination| {
            if result.discovery_urls.len() < MAX_OUTGOING_URLS_PER_PAGE {
                result.discovery_urls.push(destination.clone());
            }
            if result.internal_urls.len() < MAX_OUTGOING_URLS_PER_PAGE {
                let mut normalized = destination;
                normalized.normalize_in_place();
                if admissible_internal(&normalized, &result.url, &context)
                    && !seen.contains_key(&normalized)
                {
                    seen.insert(normalized.clone(), ());
                    result.internal_urls.push(normalized);
                }
            }
            let continue_extraction = result.discovery_urls.len() < MAX_OUTGOING_URLS_PER_PAGE
                || result.internal_urls.len() < MAX_OUTGOING_URLS_PER_PAGE;
            result.traversal_stopped = !continue_extraction;
            continue_extraction
        });
        result.discovery_limit_reached = result.discovery_urls.len() == MAX_OUTGOING_URLS_PER_PAGE;
        result.internal_limit_reached = result.internal_urls.len() == MAX_OUTGOING_URLS_PER_PAGE;
        Ok(result)
    }

    fn metadata(html: &Html) -> Self {
        Self {
            schema_version: 1,
            url: html.url().clone(),
            base_url: html.base_url().clone(),
            canonical_url: html.canonical_url(),
            no_index: html.is_no_index(),
            no_follow: html.is_no_follow(),
            discovery_urls: Vec::new(),
            internal_urls: Vec::new(),
            anchors_visited: 0,
            discovery_limit_reached: false,
            internal_limit_reached: false,
            traversal_stopped: false,
        }
    }
}

fn admissible_internal(url: &Url, document_url: &Url, context: &ExtractionContext<'_>) -> bool {
    matches!(url.scheme(), "http" | "https")
        && url.as_str().len() <= MAX_URL_LEN_BYTES
        && IGNORED_EXTENSIONS
            .iter()
            .all(|ext| !url.as_str().ends_with(ext))
        && url != document_url
        && url != context.requested_url
        && !context.crawled_urls.contains(url)
        && Domain::from(url) == *context.domain
}

/// Reference mechanism, constructible only in tests/the local benchmark.
/// Reproduce both full DOM/text passes and unbounded anchor materialization.
#[cfg(any(test, feature = "crawler-bench"))]
pub fn legacy_double_parse(
    body: &str,
    url: &Url,
    context: ExtractionContext<'_>,
) -> anyhow::Result<CrawlExtraction> {
    use itertools::Itertools;
    let local_html = Html::parse(body, url.as_str())?;
    let mut result = CrawlExtraction::metadata(&local_html);
    result.discovery_urls = local_html
        .anchor_links()
        .into_iter()
        .map(|link| link.destination)
        .take(MAX_OUTGOING_URLS_PER_PAGE)
        .collect();
    drop(local_html);
    let worker_html = Html::parse(body, url.as_str())?;
    result.internal_urls = worker_html
        .anchor_links()
        .into_iter()
        .map(|link| link.destination)
        .map(|mut url| {
            url.normalize_in_place();
            url
        })
        .filter(|url| admissible_internal(url, &result.url, &context))
        .unique()
        .take(MAX_OUTGOING_URLS_PER_PAGE)
        .collect();
    result.discovery_limit_reached = result.discovery_urls.len() == MAX_OUTGOING_URLS_PER_PAGE;
    result.internal_limit_reached = result.internal_urls.len() == MAX_OUTGOING_URLS_PER_PAGE;
    Ok(result)
}

#[derive(Default, Serialize, Clone)]
pub struct ExtractionStats {
    pub documents: u64,
    pub dom_parses: u64,
    pub failures: u64,
    pub anchors_visited: u64,
    pub discovery_links: u64,
    pub internal_links: u64,
    pub pages_at_discovery_limit: u64,
    pub pages_at_internal_limit: u64,
    pub traversals_stopped: u64,
    pub wall_us: u64,
    pub cpu_us: u64,
    pub cpu_samples: u64,
    pub max_links_per_projection: usize,
}

#[derive(Default)]
struct Counters {
    documents: AtomicU64,
    dom_parses: AtomicU64,
    failures: AtomicU64,
    anchors_visited: AtomicU64,
    discovery_links: AtomicU64,
    internal_links: AtomicU64,
    pages_at_discovery_limit: AtomicU64,
    pages_at_internal_limit: AtomicU64,
    traversals_stopped: AtomicU64,
    wall_us: AtomicU64,
    cpu_us: AtomicU64,
    cpu_samples: AtomicU64,
}

#[derive(Default, Clone)]
pub struct Extractor(Arc<Counters>);
impl Extractor {
    pub fn parse(
        &self,
        body: &str,
        url: &Url,
        context: ExtractionContext<'_>,
    ) -> anyhow::Result<CrawlExtraction> {
        let started = Instant::now();
        let cpu = thread_cpu_us();
        let result = CrawlExtraction::parse(body, url, context);
        let stats = &self.0;
        stats
            .wall_us
            .fetch_add(started.elapsed().as_micros() as u64, Ordering::Relaxed);
        if let (Some(before), Some(after)) = (cpu, thread_cpu_us()) {
            stats
                .cpu_us
                .fetch_add(after.saturating_sub(before), Ordering::Relaxed);
            stats.cpu_samples.fetch_add(1, Ordering::Relaxed);
        }
        stats.documents.fetch_add(1, Ordering::Relaxed);
        stats.dom_parses.fetch_add(1, Ordering::Relaxed);
        match &result {
            Ok(value) => {
                for (counter, count) in [
                    (&stats.anchors_visited, value.anchors_visited),
                    (&stats.discovery_links, value.discovery_urls.len() as u64),
                    (&stats.internal_links, value.internal_urls.len() as u64),
                    (
                        &stats.pages_at_discovery_limit,
                        value.discovery_limit_reached as u64,
                    ),
                    (
                        &stats.pages_at_internal_limit,
                        value.internal_limit_reached as u64,
                    ),
                    (&stats.traversals_stopped, value.traversal_stopped as u64),
                ] {
                    counter.fetch_add(count, Ordering::Relaxed);
                }
            }
            Err(_) => {
                stats.failures.fetch_add(1, Ordering::Relaxed);
            }
        }
        result
    }
    pub fn stats(&self) -> ExtractionStats {
        let c = &self.0;
        let load = |v: &AtomicU64| v.load(Ordering::Relaxed);
        ExtractionStats {
            documents: load(&c.documents),
            dom_parses: load(&c.dom_parses),
            failures: load(&c.failures),
            anchors_visited: load(&c.anchors_visited),
            discovery_links: load(&c.discovery_links),
            internal_links: load(&c.internal_links),
            pages_at_discovery_limit: load(&c.pages_at_discovery_limit),
            pages_at_internal_limit: load(&c.pages_at_internal_limit),
            traversals_stopped: load(&c.traversals_stopped),
            wall_us: load(&c.wall_us),
            cpu_us: load(&c.cpu_us),
            cpu_samples: load(&c.cpu_samples),
            max_links_per_projection: MAX_OUTGOING_URLS_PER_PAGE,
        }
    }
}

/// Actual CPU time of this synchronous extraction's thread; not elapsed time.
pub fn thread_cpu_us() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let mut time = libc::timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: a valid initialized timespec is provided to the POSIX clock.
        if unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut time) } == 0 {
            return Some(time.tv_sec as u64 * 1_000_000 + time.tv_nsec as u64 / 1_000);
        }
    }
    None
}

#[cfg(test)]
#[path = "extraction_tests.rs"]
mod tests;
