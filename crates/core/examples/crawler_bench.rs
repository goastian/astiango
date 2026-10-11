//! Run using the Python C01 orchestrator so every request has a local server ledger.
use astiango::crawler::benchmark::{self, Config};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 3 && args[1] == "--config",
        "usage: crawler_bench --config PATH"
    );
    let config: Config = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let result = benchmark::run(config).await?;
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
