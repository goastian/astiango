//! Local-only C06 CPU/allocation meter; no network or production allocator hook.
use super::extraction::{
    legacy_double_parse, thread_cpu_us, CrawlExtraction, ExtractionContext, Extractor, VisitedUrls,
};
use super::Domain;
use serde::{Deserialize, Serialize};
use std::{
    alloc::{GlobalAlloc, Layout},
    cell::Cell,
    time::Instant,
};
use url::Url;

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(false) };
    static BYTES: Cell<u64> = const { Cell::new(0) };
    static CALLS: Cell<u64> = const { Cell::new(0) };
}

/// Count requested allocation sizes, including realloc's new size, on this
/// thread only. These are cumulative allocated bytes, not peak/live bytes/RSS.
pub struct MeteredAllocator<A>(pub A);
fn allocated(size: usize, pointer: *mut u8) {
    if !pointer.is_null() && ENABLED.try_with(Cell::get).unwrap_or(false) {
        let _ = BYTES.try_with(|value| value.set(value.get().saturating_add(size as u64)));
        let _ = CALLS.try_with(|value| value.set(value.get().saturating_add(1)));
    }
}
// SAFETY: every pointer/layout is forwarded unchanged to the wrapped allocator;
// instrumentation uses allocation-free thread-local Cells and never panics.
unsafe impl<A: GlobalAlloc> GlobalAlloc for MeteredAllocator<A> {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = self.0.alloc(layout);
        allocated(layout.size(), pointer);
        pointer
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = self.0.alloc_zeroed(layout);
        allocated(layout.size(), pointer);
        pointer
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        self.0.dealloc(pointer, layout);
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = self.0.realloc(pointer, layout, size);
        allocated(size, pointer);
        pointer
    }
}
struct Scope;
impl Drop for Scope {
    fn drop(&mut self) {
        ENABLED.with(|value| value.set(false));
    }
}

#[derive(Deserialize)]
pub struct Config {
    pub mode: String,
    pub path: std::path::PathBuf,
    pub url: Url,
    pub iterations: u64,
}
#[derive(Serialize)]
pub struct Measurement {
    pub mode: String,
    pub iterations: u64,
    pub cpu_ms_per_page: f64,
    pub wall_ms_per_page: f64,
    pub allocated_bytes_per_page: f64,
    pub allocation_calls_per_page: f64,
    pub dom_parses_per_page: f64,
    pub extraction: CrawlExtraction,
}

pub fn run(config: Config) -> anyhow::Result<Measurement> {
    anyhow::ensure!(
        config.iterations > 0 && config.iterations <= 100,
        "iterations must be 1..100"
    );
    anyhow::ensure!(
        matches!(config.mode.as_str(), "legacy" | "shared"),
        "unknown extraction mode"
    );
    let bytes = std::fs::read(&config.path)?;
    let body = String::from_utf8_lossy(&bytes);
    let domain = Domain::from(&config.url);
    let visited = VisitedUrls::default();
    let context = || ExtractionContext {
        domain: &domain,
        crawled_urls: &visited,
        requested_url: &config.url,
    };
    // Warm parser selectors/regex/global caches before either timed mechanism.
    drop(legacy_double_parse(&body, &config.url, context())?);
    drop(CrawlExtraction::parse(&body, &config.url, context())?);
    let extractor = Extractor::default();
    BYTES.with(|value| value.set(0));
    CALLS.with(|value| value.set(0));
    ENABLED.with(|value| value.set(true));
    let scope = Scope;
    let cpu_before =
        thread_cpu_us().ok_or_else(|| anyhow::anyhow!("thread CPU clock unavailable"))?;
    let started = Instant::now();
    for _ in 0..config.iterations {
        let result = if config.mode == "legacy" {
            legacy_double_parse(&body, &config.url, context())?
        } else {
            extractor.parse(&body, &config.url, context())?
        };
        std::hint::black_box(&result);
        drop(result);
    }
    let wall_ms = started.elapsed().as_secs_f64() * 1000.0;
    let cpu_us = thread_cpu_us().unwrap().saturating_sub(cpu_before);
    drop(scope);
    let iterations = config.iterations as f64;
    Ok(Measurement {
        mode: config.mode.clone(),
        iterations: config.iterations,
        cpu_ms_per_page: cpu_us as f64 / 1000.0 / iterations,
        wall_ms_per_page: wall_ms / iterations,
        allocated_bytes_per_page: BYTES.with(Cell::get) as f64 / iterations,
        allocation_calls_per_page: CALLS.with(Cell::get) as f64 / iterations,
        dom_parses_per_page: if config.mode == "legacy" {
            2.0
        } else {
            extractor.stats().dom_parses as f64 / iterations
        },
        extraction: if config.mode == "legacy" {
            legacy_double_parse(&body, &config.url, context())?
        } else {
            CrawlExtraction::parse(&body, &config.url, context())?
        },
    })
}
