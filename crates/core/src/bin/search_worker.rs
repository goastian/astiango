//! A persistent native reader, owned through private bounded pipes by Search/API.
//! The parent kills and reaps this process when its request deadline expires;
//! blocking search work cannot outlive that deadline in Tokio's blocking pool.
use anyhow::{Context, Result};
use astiango::{
    entrypoint::indexer::{metrics, pipeline::generations},
    index::Index,
    searcher::{LocalSearcher, SearchQuery},
};
use clap::Parser;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    io::{BufRead, Write},
    path::PathBuf,
    sync::{atomic::Ordering, Arc},
    time::Instant,
};

struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        metrics::ALLOCATED.fetch_add(layout.size() as u64, Ordering::Relaxed);
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        System.dealloc(pointer, layout)
    }
    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        metrics::ALLOCATED.fetch_add(size as u64, Ordering::Relaxed);
        System.realloc(pointer, layout, size)
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;

#[derive(Parser)]
struct Args {
    #[arg(long)]
    root: Option<PathBuf>,
    #[arg(long)]
    generation: Option<String>,
    #[arg(long)]
    manifest_sha256: Option<String>,
    #[arg(long)]
    build_info: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: String,
    q: String,
    #[serde(default = "default_count")]
    count: usize,
    #[serde(default)]
    page: usize,
    #[serde(default)]
    safe_search: bool,
}
fn default_count() -> usize {
    10
}

fn execute(
    rt: &tokio::runtime::Runtime,
    reader: &LocalSearcher,
    request: &Request,
) -> Result<Value> {
    anyhow::ensure!(
        !request.q.trim().is_empty()
            && request.q.len() <= 4096
            && request.q.split_whitespace().count() <= 16
            && (1..=20).contains(&request.count)
            && request.page <= 20,
        "invalid query or result budget"
    );
    let start = Instant::now();
    let cpu = metrics::cpu_ns();
    let allocated = metrics::ALLOCATED.load(Ordering::Relaxed);
    let query = SearchQuery {
        query: request.q.clone(),
        num_results: request.count,
        page: request.page,
        safe_search: request.safe_search,
        ..Default::default()
    };
    let initial = rt.block_on(reader.search_initial(&query, false))?;
    let recall_ms = start.elapsed().as_secs_f64() * 1000.;
    let scores: Vec<_> = initial.websites.iter().map(|page| page.score()).collect();
    let pointers: Vec<_> = initial
        .websites
        .iter()
        .map(|page| page.pointer().clone())
        .collect();
    let retrieve = Instant::now();
    let pages = rt.block_on(reader.retrieve_websites(&pointers, &request.q))?;
    let retrieve_ms = retrieve.elapsed().as_secs_f64() * 1000.;
    anyhow::ensure!(pages.len() == pointers.len(), "incomplete native retrieval");
    let results: Vec<_> = pages.into_iter().zip(scores).map(|(page, score)| json!({
        "url":page.url, "title":page.title.chars().take(512).collect::<String>(), "snippet":if page.body.is_empty() {
            page.dirty_body.chars().take(240).collect::<String>()
        } else {page.body.chars().take(240).collect::<String>()},
        "engine_snippet":page.snippet, "region":page.region, "score":score
    })).collect();
    Ok(
        json!({"id":request.id,"outcome":"complete","results":results,
        "num_hits":initial.num_websites,"reader":"LocalSearcher",
        "native_cpu_ms":metrics::cpu_ns().saturating_sub(cpu) as f64 / 1e6,
        "allocated_bytes":metrics::ALLOCATED.load(Ordering::Relaxed).saturating_sub(allocated),
        "stages_ms":{"parse_recall_ranking":recall_ms,"retrieve_snippets":retrieve_ms},
        "native_wall_ms":start.elapsed().as_secs_f64()*1000.}),
    )
}

fn main() -> Result<()> {
    let args = Args::parse();
    if args.build_info {
        println!(
            "{}",
            json!({"version":1,"component":"search-worker","protocol":1,
            "build_id":option_env!("SEARCH_BUILD_ID").unwrap_or("unpackaged"),
            "commit":option_env!("SEARCH_GIT_COMMIT"),"lock_sha256":option_env!("SEARCH_LOCK_SHA256"),
            "generation_versions":[1],"reader":"LocalSearcher"})
        );
        return Ok(());
    }
    let root = args.root.context("--root required")?.canonicalize()?;
    let id = args.generation.context("--generation required")?;
    let started = Instant::now();
    let manifest = generations::validate_serving_generation(&root, &id)?;
    let directory = root.join("generations").join(&id);
    let digest = astiango::capture::sha256(&std::fs::read(directory.join("manifest.json"))?);
    anyhow::ensure!(
        Some(&digest) == args.manifest_sha256.as_ref(),
        "serving manifest checksum mismatch"
    );
    let mut index = Index::open(&directory)?;
    index.set_shard_id(astiango::inverted_index::ShardId::Backbone(0));
    let reader = LocalSearcher::builder(Arc::new(tokio::sync::RwLock::new(index))).build();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()?;
    let mut output = std::io::stdout().lock();
    writeln!(
        output,
        "{}",
        json!({"version":1,"state":"loaded","generation":id,
        "manifest_sha256":digest,"documents":manifest["documents"],"signals":manifest["signals"],
        "load_ms":started.elapsed().as_secs_f64()*1000.,"pid":std::process::id(),
        "capabilities":{"reader":"LocalSearcher","external_models":false,"default_ranking":true,
            "snippets":true,"max_results":20,"max_page":20}})
    )?;
    output.flush()?;
    let mut input = std::io::stdin().lock();
    loop {
        let mut line = Vec::new();
        let bytes = std::io::Read::take(&mut input, 8193).read_until(b'\n', &mut line)?;
        if bytes == 0 {
            break;
        }
        anyhow::ensure!(
            line.len() <= 8192 && line.last() == Some(&b'\n'),
            "worker message exceeds budget"
        );
        let result = match serde_json::from_slice::<Request>(&line) {
            Ok(request) => execute(&rt, &reader, &request).unwrap_or_else(
                |error| json!({"id":request.id,"outcome":"failed","error":error.to_string()}),
            ),
            Err(error) => json!({"outcome":"invalid_request","error":error.to_string()}),
        };
        let mut bytes = serde_json::to_vec(&result)?;
        anyhow::ensure!(bytes.len() <= 512 * 1024, "worker response exceeds budget");
        bytes.push(b'\n');
        output.write_all(&bytes)?;
        output.flush()?;
    }
    Ok(())
}
