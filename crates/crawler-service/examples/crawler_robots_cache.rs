use anyhow::Result;
use astiango_crawler_service::{config::CrawlNetworkPolicy, crawler::{robot_client::RobotClient, robots_txt::RobotAccess, scheduler::Mode}};

fn main() -> Result<()> {
    let ports: Vec<u16> = std::env::args().skip(1).map(|value| value.parse()).collect::<std::result::Result<_,_>>()?;
    anyhow::ensure!(ports.len() == 3, "provide three controlled loopback fixture ports");
    let config: astiango_crawler_service::crawler::local::Config = toml::from_str(&std::fs::read_to_string("configs/crawler/small.toml")?)?;
    config.validate()?;
    let mut options = config.crawler_config();
    // Only this integration example opts into private destinations. URLs below
    // are constructed from literal loopback plus supplied local server ports.
    options.network_policy = CrawlNetworkPolicy::AllowPrivate;
    options.robots.cache_entries = 2;
    options.scheduler.mode = Mode::FairTurns;
    options.min_crawl_delay_ms = 100;
    options.max_crawl_delay_ms = 300;
    options.start_politeness_factor = 1;
    options.min_politeness_factor = 1;
    let urls = ports.iter().map(|port| url::Url::parse(&format!("http://127.0.0.1:{port}/page?sig=keep%2Fexact"))).collect::<std::result::Result<Vec<_>,_>>()?;
    let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build()?;
    runtime.block_on(async {
        let client = RobotClient::new(&options)?;
        for url in &urls[..2] { anyhow::ensure!(client.robots_txt_manager().access(url).await == RobotAccess::Allowed, "fixture denied"); }
        anyhow::ensure!(matches!(client.robots_txt_manager().access(&urls[2]).await, RobotAccess::Deferred(_)), "unconsumed rules were evicted");
        anyhow::ensure!(client.robots_txt_manager().stats().entries == 2 && client.robots_txt_manager().stats().downloads == 2, "cache exceeded quota or performed unnecessary robots HTTP");
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let response = client.get(urls[0].clone()).await?.send().await?;
        anyhow::ensure!(response.status().is_success(), "page request failed");
        let text = response.text().await?;
        anyhow::ensure!(text.contains("Controlled"), "wrong fixture response");
        anyhow::ensure!(client.robots_txt_manager().access(&urls[2]).await == RobotAccess::Allowed, "cache did not progress after a page admission");
        anyhow::ensure!(client.robots_txt_manager().access(&urls[1]).await == RobotAccess::Allowed, "pending rules lost");
        let stats = client.robots_txt_manager().stats();
        anyhow::ensure!(stats.downloads == 3 && stats.entries == 2 && stats.evictions == 1 && stats.capacity_deferred == 1, "cache counters mismatch: {:?}", stats);
        client.shutdown().await;
        println!("{}",serde_json::json!({"version":1,"passed":true,"loopback_only":true,"capacity":2,"origins":3,"robots_downloads":stats.downloads,"page_downloads":1,"capacity_deferred":stats.capacity_deferred,"evictions":stats.evictions,"pending_rules_retained":true,"page_admission_progress":true}));
        Ok::<_,anyhow::Error>(())
    })
}
