#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static ALLOCATOR: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

fn main() -> anyhow::Result<()> {
    let args: Vec<_> = std::env::args().collect();
    anyhow::ensure!(
        args.len() == 3 && args[1] == "--config",
        "usage: crawler_capture_frontier_bench --config PATH"
    );
    let config = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let output = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?
        .block_on(astiango::crawler::capture_frontier_benchmark::run(config))?;
    println!("{}", serde_json::to_string(&output)?);
    Ok(())
}
