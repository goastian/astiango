//! Run via scripts/crawler-body-bench.py to retain independent server/RSS evidence.
use astiango::crawler::body_benchmark::{self, Config};
#[tokio::main(worker_threads = 8)]
async fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 3 && args[1] == "--config",
        "usage: crawler_body_bench --config PATH"
    );
    let config: Config = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    println!(
        "{}",
        serde_json::to_string(&body_benchmark::run(config).await?)?
    );
    Ok(())
}
