use anyhow::Context;
// Stract is an open source web search engine.
// Copyright (C) 2024 Stract ApS
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>

mod budgets;
mod checker;
mod crawlable_site;
mod crawled_db;
mod outbox;
mod site_url_stream;
use std::fs::File;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::config::{CheckIntervals, CrawlerConfig, LiveCrawlerConfig};
use crate::crawler::robot_client::RobotClient;
use crate::distributed::cluster::Cluster;
use crate::distributed::sonic::replication::{
    RandomReplicaSelector, RandomShardSelector, ShardedClient,
};
use crate::distributed::streaming_response::StreamingResponse;
use crate::entrypoint::search_server;
use crate::entrypoint::site_stats::FinalSiteStats;
use crate::inverted_index::ShardId;
use crate::Result;
use crate::{
    distributed::sonic::replication::ReusableShardedClient,
    entrypoint::{indexer::IndexableWebpage, live_index},
};
use crawlable_site::{CrawlableSite, CrawlableSiteGuard};
use crawled_db::ShardedCrawledDb;
use futures::StreamExt;
use site_url_stream::SiteUrlStream;
use tokio::sync::Mutex;
use tokio::sync::Semaphore;
use url::Url;

const DEFAULT_CONSISTENCY_FRACTION: f64 = 0.5;
const BLOG_FRACTION_THRESHOLD: f64 = 0.5;
const NEWS_FRACTION_THRESHOLD: f64 = 0.5;
const MIN_CRAWL_DELAY: Duration = Duration::from_secs(30);
const MAX_CRAWL_DELAY: Duration = Duration::from_secs(300);
const TICK_INTERVAL: Duration = Duration::from_secs(5);
const INDEX_ACK_TIMEOUT: Duration = Duration::from_secs(30);

fn require_index_acknowledgements<Id, E: std::fmt::Display>(
    shards: Vec<Result<(Id, Vec<(std::net::SocketAddr, Result<(), E>)>)>>,
) -> Result<()> {
    let mut acknowledgements = 0;
    for shard in shards {
        let (_, replicas) = shard?;
        anyhow::ensure!(
            !replicas.is_empty(),
            "index shard returned no acknowledgements"
        );
        for (address, reply) in replicas {
            reply.map_err(|error| {
                anyhow::anyhow!("index replica {address} rejected pages: {error}")
            })?;
            acknowledgements += 1;
        }
    }
    anyhow::ensure!(acknowledgements > 0, "index returned no acknowledgements");
    Ok(())
}

struct Client {
    live_index: Mutex<ReusableShardedClient<live_index::LiveIndexService>>,
    search: Mutex<ReusableShardedClient<search_server::SearchService>>,
    client: RobotClient,
    outbox: Option<Arc<outbox::Outbox>>,
}

impl Client {
    pub async fn new(cluster: Arc<Cluster>, crawler_config: &CrawlerConfig) -> Result<Self> {
        let live_index = Mutex::new(ReusableShardedClient::new(cluster.clone()).await);
        let search = Mutex::new(ReusableShardedClient::new(cluster.clone()).await);
        let client = RobotClient::new(crawler_config)?;

        Ok(Self {
            client,
            live_index,
            search,
            outbox: None,
        })
    }

    pub fn reqwest(&self) -> RobotClient {
        self.client.clone()
    }

    async fn live_conn(&self) -> Arc<ShardedClient<live_index::LiveIndexService, ShardId>> {
        self.live_index.lock().await.conn().await
    }

    pub async fn index(&self, pages: Vec<IndexableWebpage>) -> Result<()> {
        tokio::time::timeout(INDEX_ACK_TIMEOUT, async {
            let conn = self.live_conn().await;
            let req = live_index::IndexWebpages {
                pages,
                consistency_fraction: Some(DEFAULT_CONSISTENCY_FRACTION),
            };
            let replies = conn
                .send_with_timeout(
                    req,
                    &RandomShardSelector,
                    &RandomReplicaSelector,
                    INDEX_ACK_TIMEOUT,
                )
                .await?;
            require_index_acknowledgements(
                replies
                    .into_iter()
                    .map(|reply| {
                        reply.map_err(|error| anyhow::anyhow!("index shard failed: {error}"))
                    })
                    .collect(),
            )
        })
        .await
        .map_err(|_| anyhow::anyhow!("index acknowledgement timed out"))?
    }

    pub async fn get_site_urls(&self, site: &str) -> Result<Vec<Url>> {
        let mut res = Vec::new();
        let mut stream =
            SiteUrlStream::new(site.to_string(), self.search.lock().await.clone()).stream();

        while let Some(url) = stream.next().await {
            res.push(url?);
        }

        Ok(res)
    }
}

impl From<LiveCrawlerConfig> for CrawlerConfig {
    fn from(config: LiveCrawlerConfig) -> Self {
        Self {
            num_worker_threads: 1,
            discovery: config.discovery,
            retry: config.retry,
            recrawl: crate::crawler::recrawl::Config::default(),
            scheduler: crate::crawler::scheduler::Config::default(),
            robots: crate::crawler::robots_txt::Config::default(),
            user_agent: config.user_agent,
            network_policy: config.network_policy,
            transport: config.transport,
            body: config.body,
            pipeline: config.pipeline,
            spool: crate::crawler::spool::SpoolConfig::default(),
            max_redirect_hops: config.max_redirect_hops,
            robots_txt_cache_sec: crate::config::defaults::Crawler::robots_txt_cache_sec(),
            min_politeness_factor: 0,
            start_politeness_factor: 1,
            min_crawl_delay_ms: MIN_CRAWL_DELAY.as_millis() as u64,
            max_crawl_delay_ms: MAX_CRAWL_DELAY.as_millis() as u64,
            max_politeness_factor: crate::config::defaults::Crawler::max_politeness_factor(),
            max_url_slowdown_retry: crate::config::defaults::Crawler::max_url_slowdown_retry(),
            timeout_seconds: 60,
            s3: crate::config::S3Config {
                bucket: String::new(),
                folder: String::new(),
                access_key: String::new(),
                secret_key: String::new(),
                endpoint: String::new(),
            },
            router_hosts: vec![],
        }
    }
}

struct SiteStats {
    sites: Vec<FinalSiteStats>,
}

impl SiteStats {
    pub fn open(path: PathBuf) -> Result<Self> {
        let file = File::open(path)?;
        let sites = serde_json::from_reader(file)?;
        Ok(Self { sites })
    }

    pub fn blogs(&self) -> impl Iterator<Item = &FinalSiteStats> {
        self.sites.iter().filter(|site| {
            site.stats().blogposts as f64 / site.stats().pages as f64 > BLOG_FRACTION_THRESHOLD
        })
    }

    pub fn news(&self) -> impl Iterator<Item = &FinalSiteStats> {
        self.sites.iter().filter(|site| {
            site.stats().news_articles as f64 / site.stats().pages as f64 > NEWS_FRACTION_THRESHOLD
        })
    }

    pub fn all(&self) -> impl Iterator<Item = &FinalSiteStats> {
        self.sites.iter()
    }
}

pub struct Crawler {
    client: Arc<Client>,
    db: Arc<ShardedCrawledDb>,
    sites: Vec<Arc<CrawlableSite>>,
    num_worker_threads: usize,
    check_intervals: CheckIntervals,
    crawler_config: Arc<CrawlerConfig>,
}

impl Crawler {
    pub async fn new(config: LiveCrawlerConfig) -> Result<Self> {
        let crawler_config = Arc::new(CrawlerConfig::from(config.clone()));

        let cluster = Arc::new(
            Cluster::join_as_spectator(
                config.gossip.addr,
                config.gossip.seed_nodes.unwrap_or_default(),
            )
            .await?,
        );

        let mut client = Client::new(cluster, &crawler_config).await?;
        std::fs::create_dir_all(&config.crawled_db_path)?;
        let discovery = Arc::new(crate::crawler::frontier::Frontier::open_database(
            config.crawled_db_path.join("discovery.redb"),
            10_000,
            32,
        )?);
        discovery.configure_discovery(&crawler_config.discovery, &Default::default())?;
        discovery.configure_retry(&crawler_config.retry)?;
        client.client = client.client.with_discovery_frontier(discovery);
        client.outbox = Some(Arc::new(outbox::Outbox::open(
            &config.crawled_db_path.join("index-outbox.redb"),
        )?));
        let client = Arc::new(client);
        let db = Arc::new(ShardedCrawledDb::open(config.crawled_db_path)?);

        let site_stats = SiteStats::open(config.site_stats_path)?;
        let sites: Vec<_> = site_stats.all().cloned().collect();

        let budgets = budgets::SiteBudgets::new(
            &config.host_centrality_path,
            &site_stats,
            config.daily_budget.clone(),
        )?;

        let mut crawlable_sites = Vec::new();
        if config.init_crawl_db {
            tracing::debug!("Initializing crawler db with previously crawled urls");
            for site in &sites {
                for url in client.get_site_urls(site.site().as_str()).await? {
                    if !db.has_crawled(&url)? {
                        db.insert(&url)?;
                    }
                }
            }
        }

        for site in sites {
            if let Some(drip_rate) = budgets.drip_rate(site.site()) {
                crawlable_sites.push(Arc::new(CrawlableSite::new(site, &client, drip_rate)?));
            }
        }

        Ok(Self {
            client,
            db,
            sites: crawlable_sites,
            num_worker_threads: config.num_worker_threads,
            check_intervals: config.check_intervals,
            crawler_config,
        })
    }

    pub async fn run(self) -> Result<()> {
        let semaphore = Arc::new(Semaphore::new(self.num_worker_threads.max(1)));
        let mut ready = std::collections::BinaryHeap::new();
        for (index, site) in self.sites.iter().enumerate() {
            ready.push(std::cmp::Reverse((
                site.next_eligible(&self.check_intervals).await,
                index,
            )));
        }
        let mut tasks: tokio::task::JoinSet<(usize, Result<()>)> = tokio::task::JoinSet::new();
        let mut maintenance = tokio::time::interval(TICK_INTERVAL);
        let mut publications = tokio::task::JoinSet::<Result<()>>::new();
        loop {
            let due = ready
                .peek()
                .map(|v| v.0 .0)
                .unwrap_or_else(|| std::time::Instant::now() + TICK_INTERVAL);
            tokio::select! {
                result = tasks.join_next(), if !tasks.is_empty() => {
                    let (index, outcome) = result.ok_or_else(|| anyhow::anyhow!("live worker result missing"))??;
                    if let Err(error) = outcome { tracing::error!("Live crawl incomplete: {error}"); }
                    ready.push(std::cmp::Reverse((self.sites[index].next_eligible(&self.check_intervals).await, index)));
                }
                _ = tokio::time::sleep_until(due.into()), if !ready.is_empty() && semaphore.available_permits() > 0 => {
                    let permit = semaphore.clone().try_acquire_owned()?;
                    let std::cmp::Reverse((_, index)) = ready.pop().unwrap();
                    let site = self.sites[index].clone();
                    site.drip().await;
                    if !site.should_crawl(&self.check_intervals).await {
                        ready.push(std::cmp::Reverse((site.next_eligible(&self.check_intervals).await, index)));
                        continue;
                    }
                    let guard = CrawlableSiteGuard::new(site, self.db.clone(), self.crawler_config.clone()).await;
                    let client = self.client.clone(); let intervals = self.check_intervals.clone();
                    tasks.spawn(async move { let _permit = permit; (index, guard.crawl(&client, &intervals).await) });
                }
                result = publications.join_next(), if !publications.is_empty() => {
                    result.context("live outbox result missing")??.inspect_err(|e| tracing::error!("live outbox publication failed: {e}"))?;
                }
                _ = maintenance.tick(), if publications.is_empty() => {
                    if let Some(outbox) = &self.client.outbox {
                        let disk = outbox.clone(); let client = self.client.clone(); let db = self.db.clone();
                        publications.spawn(async move {
                            let now = chrono::Utc::now().timestamp_millis().max(0) as u64;
                            let read_disk = disk.clone();
                            let entry = client.client.pipeline().control.run(0, move || read_disk.ready(now)).await??;
                            if let Some(entry) = entry {
                                let result = client.index(if entry.datum.indexing_allowed() { vec![IndexableWebpage::from(entry.datum.clone())] } else { Vec::new() }).await;
                                let frontier = client.client.discovery_frontier();
                                client.client.pipeline().control.run_durable(entry.datum.memory_bytes(), move || -> Result<()> {
                                    match result {
                                        Ok(()) => {
                                            db.insert(&entry.datum.url)?;
                                            if let Some(capture) = &entry.datum.capture {
                                                let original = Url::parse(&capture.metadata.requested_url)?;
                                                db.insert(&original)?;
                                                if let Some(frontier) = &frontier { frontier.settle_live_candidate(&original, None)?; }
                                            }
                                            disk.acknowledge(&entry.id)?;
                                        }
                                        Err(error) => disk.defer(&entry.id, now, error.to_string())?,
                                    }
                                    Ok(())
                                }).await??;
                            }
                            Ok(())
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod acknowledgement_tests {
    use super::*;

    type Reply = Result<(u64, Vec<(std::net::SocketAddr, Result<(), &'static str>)>)>;

    #[test]
    fn indexing_requires_an_application_acknowledgement() {
        assert!(require_index_acknowledgements::<u64, &str>(Vec::new()).is_err());
        let empty: Vec<Reply> = vec![Ok((0, Vec::new()))];
        assert!(require_index_acknowledgements(empty).is_err());
        let acknowledged: Vec<Reply> =
            vec![Ok((0, vec![("127.0.0.1:1".parse().unwrap(), Ok(()))]))];
        require_index_acknowledgements(acknowledged).unwrap();
    }

    #[test]
    fn indexing_propagates_transport_and_nested_application_failures() {
        let transport: Vec<Reply> = vec![Err(anyhow::anyhow!("connection lost"))];
        assert!(require_index_acknowledgements(transport)
            .unwrap_err()
            .to_string()
            .contains("connection lost"));
        let rejected: Vec<Reply> = vec![Ok((
            0,
            vec![(
                "127.0.0.1:1".parse().unwrap(),
                Err("insufficient replication"),
            )],
        ))];
        assert!(require_index_acknowledgements(rejected)
            .unwrap_err()
            .to_string()
            .contains("insufficient replication"));
    }
}
