#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: crawler_scheduler_bench CONFIG.json"))?;
    let config = serde_json::from_slice(&std::fs::read(path)?)?;
    println!(
        "{}",
        astiango::crawler::scheduler_benchmark::run(config).await?
    );
    Ok(())
}
