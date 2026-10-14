use astiango::webgraph::{metrics, pipeline};
use clap::Parser;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    path::PathBuf,
    sync::atomic::Ordering,
};
struct Allocator;
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        metrics::ALLOCATED.fetch_add(l.size() as u64, Ordering::Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        System.dealloc(p, l)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        metrics::ALLOCATED.fetch_add(n as u64, Ordering::Relaxed);
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static ALLOCATOR: Allocator = Allocator;
#[derive(Parser)]
struct Args {
    config: Option<PathBuf>,
    #[arg(long)]
    prepare_generation: bool,
    #[arg(long)]
    rebuild: bool,
    #[arg(long)]
    repair: bool,
    #[arg(long)]
    audit: bool,
    #[arg(long)]
    query: Option<String>,
    #[arg(long)]
    backlinks: bool,
    #[arg(long)]
    build_info: bool,
    #[arg(long)]
    make_corpus: Option<PathBuf>,
    #[arg(long)]
    legacy_baseline: bool,
    #[arg(long)]
    compare_centrality: Option<PathBuf>,
    #[arg(long)]
    inspect_signals: Option<PathBuf>,
    #[arg(long)]
    rollback: bool,
    #[arg(long)]
    planner_url: Vec<String>,
}
extern "C" fn drain(_: libc::c_int) {
    pipeline::CANCELLED.store(true, Ordering::Relaxed);
}
fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    unsafe {
        libc::signal(libc::SIGTERM, drain as *const () as usize);
        libc::signal(libc::SIGINT, drain as *const () as usize);
    }
    let args = Args::parse();
    if args.build_info {
        println!(
            "{}",
            serde_json::json!({"version":1,"build_id":option_env!("WEBGRAPH_BUILD_ID").unwrap_or("unpackaged"),"commit":option_env!("WEBGRAPH_GIT_COMMIT"),"ledger_version":1,"generation_version":2,"generation_versions":[1,2],"signal_version":2,"engine":"tantivy-fork-0.23 graph-schema-v1","identity":"C11 ledger + legacy NodeID projection","lock_sha256":option_env!("WEBGRAPH_LOCK_SHA256")})
        );
        return Ok(());
    }
    if let Some(path) = args.make_corpus {
        println!("{}", pipeline::fixture::create(&path)?);
        return Ok(());
    }
    let raw = std::fs::read_to_string(args.config.context("graph config required")?)?;
    metrics::ENABLED.store(true, Ordering::Relaxed);
    if args.legacy_baseline {
        let config: astiango::config::WebgraphConstructConfig = toml::from_str(&raw)?;
        let start = std::time::Instant::now();
        let cpu = metrics::cpu_ns();
        astiango::entrypoint::Webgraph::run(&config)?;
        let graph = astiango::webgraph::Webgraph::open(&config.graph_base_path, 0u64.into())?;
        println!(
            "{}",
            serde_json::json!({"version":1,"current_edges":graph.page_edges().count(),"elapsed_ms":start.elapsed().as_secs_f64()*1000.0,"cpu_ms":metrics::cpu_ns().saturating_sub(cpu) as f64/1e6,"allocated_bytes":metrics::ALLOCATED.load(Ordering::Relaxed),"stages":metrics::snapshot()})
        );
        return Ok(());
    }
    let config: pipeline::Config = toml::from_str(&raw)?;
    config.validate()?;
    let value = if let Some(path) = args.inspect_signals {
        if args.planner_url.is_empty() {
            astiango::webgraph::signals::Bundle::load(&path)?.binding
        } else {
            astiango::crawler::planner::preview_signals(&path, &args.planner_url)?
        }
    } else if let Some(path) = args.compare_centrality {
        astiango::webgraph::signals::compare(&path, &config)?
    } else if args.rollback {
        pipeline::rollback(&config)?
    } else if args.prepare_generation {
        let cpu = metrics::cpu_ns();
        let mut v = serde_json::to_value(pipeline::prepare(&config, args.rebuild)?)?;
        v["stages"] = serde_json::to_value(metrics::snapshot())?;
        v["cpu_ms"] = serde_json::json!(metrics::cpu_ns().saturating_sub(cpu) as f64 / 1e6);
        v["allocated_bytes"] = serde_json::json!(metrics::ALLOCATED.load(Ordering::Relaxed));
        v
    } else if args.repair {
        pipeline::repair(&config)?;
        serde_json::json!({"repaired":true})
    } else if args.audit {
        pipeline::audit(&config)?
    } else if let Some(query) = args.query {
        pipeline::query(&config, &query, args.backlinks)?
    } else {
        serde_json::to_value(pipeline::run(&config)?)?
    };
    println!("{}", value);
    Ok(())
}
use anyhow::Context;
