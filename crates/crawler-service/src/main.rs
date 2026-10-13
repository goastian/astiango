use anyhow::{Context, Result};
use astiango_crawler_service as astiango;
use clap::{Parser, Subcommand};
use serde::de::DeserializeOwned;

#[derive(Parser)]
#[command(
    name = "astiango-crawler",
    version,
    about = "Crawler acquisition and durable capture service"
)]
struct Args {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Normalize,
    Run { config: String },
    Validate { config: String },
    LeasedWorker { config: String },
    BuildInfo,
}

fn load<T: DeserializeOwned>(path: &str) -> Result<T> {
    toml::from_str(&std::fs::read_to_string(path).with_context(|| format!("cannot read {path}"))?)
        .with_context(|| format!("invalid configuration: {path}"))
}

#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
    let args = Args::parse();
    if matches!(args.command, Command::Normalize) {
        use std::io::Read;
        let mut input = String::new();
        std::io::stdin()
            .take(2 * 1024 * 1024 + 1)
            .read_to_string(&mut input)?;
        anyhow::ensure!(
            input.len() <= 2 * 1024 * 1024,
            "URL identity message exceeds budget"
        );
        let urls: Vec<String> = serde_json::from_str(&input)?;
        anyhow::ensure!(urls.len() <= 128, "URL identity batch exceeds budget");
        let rows: Result<Vec<_>> = urls.into_iter().map(|value| {
            let url = url::Url::parse(&value)?;
            let exact = astiango::capture::url_key(&url)?;
            anyhow::ensure!(exact.len() <= astiango::crawler::MAX_URL_LEN_BYTES, "URL exceeds identity budget");
            let origin = url.origin().ascii_serialization();
            let digest = ring::digest::digest(&ring::digest::SHA256, origin.as_bytes());
            let partition = u64::from_be_bytes(digest.as_ref()[..8].try_into()?) % 128;
            Ok(serde_json::json!({"id":astiango::capture::sha256(exact.as_bytes()),"url":exact,"origin":origin,"partition_id":partition}))
        }).collect();
        println!("{}", serde_json::json!({"identity_version":1,"urls":rows?}));
        return Ok(());
    }
    if matches!(args.command, Command::BuildInfo) {
        println!(
            "{}",
            serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"commit":option_env!("CRAWLER_BUILD_COMMIT"),
            "source_sha256":option_env!("CRAWLER_SOURCE_SHA256"),"rustc":option_env!("CRAWLER_RUSTC"),"embedded_frontier":"redb",
            "core_dependency_isolated":cfg!(crawler_standalone)})
        );
        return Ok(());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(astiango::crawler::resources::effective_cpus().min(4))
        .enable_all()
        .build()?;
    match args.command {
        Command::Validate { config } => {
            let config: astiango::crawler::local::Config = load(&config)?;
            config.validate()?;
            println!("Crawler configuration is valid");
        }
        Command::Run { config } => {
            let config: astiango::crawler::local::Config = load(&config)?;
            config.validate()?;
            runtime.block_on(astiango::crawler::local::run(config))?;
        }
        Command::LeasedWorker { config } => {
            runtime.block_on(astiango::crawler::leased::run(load(&config)?))?
        }
        Command::BuildInfo => unreachable!(),
        Command::Normalize => unreachable!(),
    }
    Ok(())
}
