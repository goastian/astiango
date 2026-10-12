//! Run through scripts/crawler-transport-bench.py for the independent socket ledger.
use astiango::crawler::transport_benchmark::{self, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 3 && args[1] == "--config",
        "usage: crawler_transport_bench --config PATH"
    );
    let config: Config = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let result = transport_benchmark::run(config).await?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
