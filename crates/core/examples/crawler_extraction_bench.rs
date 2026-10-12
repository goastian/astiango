use astiango::crawler::extraction_benchmark::{self, MeteredAllocator};
#[cfg(not(target_env = "msvc"))]
#[global_allocator]
static ALLOCATOR: MeteredAllocator<tikv_jemallocator::Jemalloc> =
    MeteredAllocator(tikv_jemallocator::Jemalloc);
#[cfg(target_env = "msvc")]
#[global_allocator]
static ALLOCATOR: MeteredAllocator<std::alloc::System> = MeteredAllocator(std::alloc::System);
fn main() -> anyhow::Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    anyhow::ensure!(
        args.len() == 3 && args[1] == "--config",
        "usage: crawler_extraction_bench --config PATH"
    );
    let config = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    println!(
        "{}",
        serde_json::to_string(&extraction_benchmark::run(config)?)?
    );
    Ok(())
}
