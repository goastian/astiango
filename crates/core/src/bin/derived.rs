//! Dedicated, bounded specialized-document consumer; produces a normal index shard.
use anyhow::Result;
use clap::Parser;
#[derive(Parser)]
#[command(about="Index versioned specialized derivatives into a mergeable search shard")]
struct Args { input: std::path::PathBuf, output: std::path::PathBuf }
fn main() -> Result<()> {
    tracing_subscriber::fmt().with_writer(std::io::stderr).init();
    let args=Args::parse();
    let report=astiango::entrypoint::indexer::derived::run(&args.input,&args.output)?;
    println!("{}",serde_json::to_string(&report)?);
    Ok(())
}
