use astiango::entrypoint::indexer::{self, metrics, pipeline};
use clap::Parser;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    path::PathBuf,
    sync::atomic::Ordering,
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
    config: Option<PathBuf>,
    #[arg(long)]
    make_corpus: Option<PathBuf>,
    #[arg(long)]
    legacy: bool,
    #[arg(long)]
    query: Option<String>,
    #[arg(long)]
    audit: bool,
    #[arg(long)]
    prepare_generation: bool,
    #[arg(long)]
    source_directory: Option<PathBuf>,
    #[arg(long)]
    validate_query: Option<String>,
    #[arg(long)]
    promote: Option<String>,
    #[arg(long)]
    rollback: bool,
    #[arg(long)]
    repair_publication: bool,
    #[arg(long)]
    generation_query: Option<String>,
    #[arg(long)]
    build_info: bool,
}
fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    unsafe {
        libc::signal(libc::SIGTERM, cancel as *const () as usize);
        libc::signal(libc::SIGINT, cancel as *const () as usize);
    }
    let args = Args::parse();
    if args.build_info {
        println!("{}", pipeline::generations::build_info());
        return Ok(());
    }
    if let Some(path) = args.make_corpus {
        println!("{}", indexer::fixture::create(&path)?);
        return Ok(());
    }
    let raw = std::fs::read_to_string(
        args.config
            .ok_or_else(|| anyhow::anyhow!("config path required"))?,
    )?;
    let mut value: toml::Value = toml::from_str(&raw)?;
    let policy: pipeline::Config = value
        .as_table_mut()
        .unwrap()
        .remove("ingestion")
        .map(|v| v.try_into())
        .transpose()?
        .unwrap_or_default();
    let known = [
        "consumer",
        "dedup",
        "output_path",
        "limit_warc_files",
        "skip_warc_files",
        "warc_source",
        "page_webgraph",
        "host_centrality_threshold",
        "host_centrality_store_path",
        "page_centrality_store_path",
        "safety_classifier_path",
        "minimum_clean_words",
        "batch_size",
        "autocommit_after_num_inserts",
        "dual_encoder",
    ];
    anyhow::ensure!(
        value
            .as_table()
            .unwrap()
            .keys()
            .all(|key| known.contains(&key.as_str())),
        "unknown indexer configuration option"
    );
    let config: astiango::config::IndexerConfig = value.try_into()?;
    policy.validate(&config)?;
    let operations = [
        args.audit,
        args.query.is_some(),
        args.prepare_generation,
        args.promote.is_some(),
        args.rollback,
        args.repair_publication,
        args.generation_query.is_some(),
        args.legacy,
    ];
    anyhow::ensure!(
        operations.into_iter().filter(|v| *v).count() <= 1,
        "choose one indexer operation"
    );
    anyhow::ensure!(
        args.prepare_generation
            || (args.source_directory.is_none() && args.validate_query.is_none()),
        "generation options require --prepare-generation"
    );
    let result = if args.prepare_generation {
        Some(pipeline::generations::prepare(
            &config,
            &policy,
            args.source_directory.as_deref(),
            args.validate_query.as_deref(),
        )?)
    } else if let Some(id) = args.promote {
        Some(pipeline::generations::promote(&config, &id, false)?)
    } else if args.rollback {
        Some(pipeline::generations::promote(&config, "", true)?)
    } else if args.repair_publication {
        Some(pipeline::generations::repair(&config)?)
    } else if let Some(query) = args.generation_query {
        Some(pipeline::generations::query(&config, &query)?)
    } else {
        None
    };
    if let Some(result) = result {
        println!("{}", result);
        return Ok(());
    }
    let output = std::path::Path::new(&config.output_path);
    if args.legacy {
        anyhow::ensure!(
            !output.join("ledger.redb").exists(),
            "legacy mode requires a separate output; shared ledger retained"
        );
    } else {
        anyhow::ensure!(
            !output.join("inverted_index").exists(),
            "shared ingestion requires a separate output from a legacy generation"
        );
    }
    if args.audit || args.query.is_some() {
        anyhow::ensure!(
            output.join("ledger.redb").is_file()
                && output.join("working/inverted_index/meta.json").is_file(),
            "no committed shared index available for inspection"
        );
    }
    metrics::ENABLED.store(true, Ordering::Relaxed);
    if args.audit {
        println!("{}", pipeline::audit(&config)?);
        return Ok(());
    }
    if let Some(query) = args.query {
        println!("{}", pipeline::preview(&config, &query)?);
        return Ok(());
    }
    if args.legacy {
        let start = Instant::now();
        let cpu = metrics::cpu_ns();
        let allocated = metrics::ALLOCATED.load(Ordering::Relaxed);
        indexer::run(&config)?;
        let index = astiango::index::Index::open(&config.output_path)?;
        println!(
            "{}",
            serde_json::json!({"version":1,"mode":"legacy","documents":index.inverted_index.num_documents(),"elapsed_ms":start.elapsed().as_secs_f64()*1000.,"cpu_ms":(metrics::cpu_ns()-cpu) as f64/1e6,"allocated_bytes":metrics::ALLOCATED.load(Ordering::Relaxed)-allocated,"stages":metrics::snapshot()})
        );
    } else {
        let report = pipeline::run(&config, &policy)?;
        println!("{}", serde_json::to_string(&report)?);
        if policy.publish_generations {
            let publication = (|| -> anyhow::Result<()> {
                let manifest = pipeline::generations::prepare(&config, &policy, None, None)?;
                let id = manifest["id"]
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("missing generation ID"))?;
                pipeline::generations::promote(&config, id, false)?;
                Ok(())
            })();
            if let Err(error) = publication {
                pipeline::generations::record_publication_error(&config, &error.to_string())?;
                tracing::error!(%error, "ingestion committed; generation publication retained for retry");
            }
        }
    }
    Ok(())
}

extern "C" fn cancel(_: libc::c_int) {
    pipeline::CANCELLED.store(true, Ordering::Relaxed);
}
