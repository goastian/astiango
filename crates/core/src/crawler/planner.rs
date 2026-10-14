// Stract is an open source web search engine.
// Copyright (C) 2023 Stract ApS
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
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use anyhow::{Context, Result};
use futures::stream::FuturesOrdered;
use futures::StreamExt;
use indicatif::ProgressIterator;
use itertools::Itertools;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use url::Url;

use crate::crawler::WeightedUrl;
use crate::distributed::cluster::Cluster;
use crate::external_sort::ExternalSorter;
use crate::log_group::HarmonicRankGroup;
use crate::webgraph::query::Id2NodeQuery;
use crate::webgraph::remote::RemoteWebgraph;
use crate::webgraph::Node;
use crate::webpage::url_ext::UrlExt;
use crate::SortableFloat;
use crate::{
    config::CrawlPlannerConfig,
    crawler::{file_queue::FileQueueWriter, Job},
    webgraph::NodeID,
};

use super::Domain;

const NUM_GROUPS: usize = 1024;
const CONCURRENCY_LIMIT: usize = 32;
pub(super) fn select_group(complete: &[bool], ordinal: u64, exploration: f64) -> Option<usize> {
    let high = complete.iter().position(|done| !done)?;
    let period = if exploration > 0.0 {
        (1.0 / exploration).round().max(1.0) as u64
    } else {
        u64::MAX
    };
    Some(if ordinal % period == period - 1 {
        complete.iter().rposition(|done| !done).unwrap()
    } else {
        high
    })
}
fn source_digest(path: &Path) -> Result<String> {
    fn visit(path: &Path, digest: &mut ring::digest::Context) -> Result<()> {
        use std::io::Read;
        let metadata = std::fs::symlink_metadata(path)?;
        anyhow::ensure!(
            !metadata.file_type().is_symlink(),
            "planner input symlinks are not supported"
        );
        if metadata.is_dir() {
            let mut entries = std::fs::read_dir(path)?
                .map(|entry| entry.map(|e| e.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            entries.sort();
            for entry in entries {
                digest.update(entry.file_name().unwrap().as_encoded_bytes());
                digest.update(b"\0");
                visit(&entry, digest)?;
            }
        } else {
            digest.update(&metadata.len().to_le_bytes());
            let mut file = std::fs::File::open(path)?;
            let mut buffer = [0; 8192];
            loop {
                let n = file.read(&mut buffer)?;
                if n == 0 {
                    break;
                }
                digest.update(&buffer[..n]);
            }
        }
        Ok(())
    }
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    visit(path, &mut digest)?;
    Ok(crate::capture::hex(digest.finish().as_ref()))
}
/// Read the same validated stores and grouping rule used by the planner, without admitting HTTP.
pub fn preview_signals(path: &std::path::Path, urls: &[String]) -> Result<serde_json::Value> {
    let bundle = crate::webgraph::signals::Bundle::load(path)?;
    anyhow::ensure!(
        bundle.scope != "page",
        "host planner cannot adopt page-scope centrality"
    );
    let ranks = speedy_kv::Db::<NodeID, u64>::open_or_create(path.join("scores/harmonic_rank"))?;
    let groups = HarmonicRankGroup::new(ranks.len() as u64, NUM_GROUPS as u64);
    let mut results = Vec::new();
    for raw in urls {
        let url = Url::parse(raw)?;
        let domain = crate::webgraph::signals::key(&url, &bundle.scope)?;
        let id = Node::from(Url::parse(&format!("https://{domain}/"))?)
            .into_host()
            .id();
        let rank = ranks.get(&id)?;
        results.push(serde_json::json!({"url":raw,"identity":domain,"rank":rank,"group":rank_group(rank,&groups),"score":bundle.lookup(&url)?.0}));
    }
    Ok(
        serde_json::json!({"binding":bundle.binding,"results":results,"adopted":false,"external_requests":0}),
    )
}
fn rank_group(rank: Option<u64>, groups: &HarmonicRankGroup) -> usize {
    rank.map(|rank| NUM_GROUPS - 1 - groups.group(rank) as usize)
        .unwrap_or(NUM_GROUPS - 1)
        .min(NUM_GROUPS - 1)
}
struct UrlGrouper {
    store: Arc<super::plan_store::Store>,
    harmonic_group: Arc<HarmonicRankGroup>,
    harmonic_rank: Arc<speedy_kv::Db<NodeID, u64>>,
    signal_scope: Option<String>,
}
impl UrlGrouper {
    fn group(&self, url: &Url) -> usize {
        self.group_checked(url).unwrap_or(NUM_GROUPS - 1)
    }
    fn group_checked(&self, url: &Url) -> Result<usize> {
        let owned = self
            .signal_scope
            .as_ref()
            .map(|scope| crate::webgraph::signals::key(url, scope))
            .transpose()?;
        let Some(domain) = owned.as_deref().or_else(|| url.icann_domain()) else {
            return Ok(NUM_GROUPS - 1);
        };
        let Ok(domain_url) = Url::parse(&format!("https://{domain}/")) else {
            return Ok(NUM_GROUPS - 1);
        };
        let id = Node::from(domain_url).into_host().id();
        Ok(rank_group(
            self.harmonic_rank.get(&id)?,
            &self.harmonic_group,
        ))
    }
    fn insert(&self, url: Url, host: NodeID) -> Result<bool> {
        self.store
            .insert(self.group_checked(&url)? as u64, &url, host.as_u128())
    }
    fn into_groups(self) -> Vec<super::plan_store::Group> {
        (0..NUM_GROUPS as u64)
            .map(|id| self.store.group(id))
            .collect()
    }
}

fn sorted_centralities(
    it: impl Iterator<Item = (NodeID, f64)>,
    max_bytes: usize,
) -> impl Iterator<Item = (NodeID, f64)> {
    ExternalSorter::new()
        .with_chunk_size(max_bytes / std::mem::size_of::<(NodeID, f64)>())
        .sort(it.map(|(node_id, centrality)| (Reverse(SortableFloat(centrality)), node_id)))
        .unwrap()
        .map(|(Reverse(SortableFloat(centrality)), node_id)| (node_id, centrality))
}

#[derive(Debug, Clone)]
struct Budget {
    remaining_schedulable: u64,
}

/// Create a crawl plan based on the harmonic rank of the hosts.
pub struct CrawlPlanner {
    host_centrality: Arc<speedy_kv::Db<NodeID, f64>>,
    host_centrality_rank: Arc<speedy_kv::Db<NodeID, u64>>,
    page_centrality: Arc<speedy_kv::Db<NodeID, f64>>,
    webgraph: RemoteWebgraph,
    config: CrawlPlannerConfig,

    harmonic_group: Arc<HarmonicRankGroup>,

    excluded_domains: HashSet<Domain>,

    domain_boosts: HashMap<Domain, f64>,
}

impl CrawlPlanner {
    pub async fn new(
        host_centrality: speedy_kv::Db<NodeID, f64>,
        host_centrality_rank: speedy_kv::Db<NodeID, u64>,
        page_centrality: speedy_kv::Db<NodeID, f64>,
        cluster: Arc<Cluster>,
        config: CrawlPlannerConfig,
    ) -> Result<Self> {
        Self::check_config(&config)?;

        let webgraph = RemoteWebgraph::new(cluster.clone()).await;
        webgraph.await_ready().await;

        let domain_boosts = config
            .domain_boosts
            .clone()
            .unwrap_or_default()
            .into_iter()
            .map(|domain_boost| (Domain(domain_boost.domain), domain_boost.boost))
            .collect();

        Ok(Self {
            host_centrality: Arc::new(host_centrality),
            page_centrality: Arc::new(page_centrality),
            webgraph,
            domain_boosts,
            excluded_domains: config
                .excluded_domains
                .clone()
                .unwrap_or_default()
                .into_iter()
                .map(Domain)
                .collect(),
            harmonic_group: Arc::new(HarmonicRankGroup::new(
                host_centrality_rank.len() as u64,
                NUM_GROUPS as u64,
            )),
            host_centrality_rank: Arc::new(host_centrality_rank),
            config,
        })
    }

    fn check_config(config: &CrawlPlannerConfig) -> Result<()> {
        if let Some(path) = &config.graph_generation {
            let bundle = crate::webgraph::signals::Bundle::load(path)?;
            anyhow::ensure!(
                bundle.scope != "page",
                "planner host ranking requires host or tenant signals"
            );
            let scores = bundle.path.join("scores").canonicalize()?;
            anyhow::ensure!(
                Path::new(&config.host_harmonic_path).canonicalize()? == scores
                    && Path::new(&config.host_centrality_rank_store_path).canonicalize()?
                        == scores.join("harmonic_rank").canonicalize()?,
                "planner graph/scores/ranks generation mismatch"
            );
        }
        config.signals.validate()?;
        anyhow::ensure!(
            config.num_job_queues > 0
                && config.max_urls_per_job > 0
                && config.max_urls_per_job <= 128
                && config.job_bytes >= 16384
                && config.job_bytes <= config.sort_memory_bytes
                && config.sort_memory_bytes >= 65536
                && (0.0..=0.5).contains(&config.exploration_fraction),
            "invalid planner resource budgets"
        );
        if !(0.0..=1.0).contains(&config.wander_fraction) {
            return Err(anyhow::anyhow!(
                "top_host_fraction must be in range [0.0, 1.0]"
            ));
        }

        Ok(())
    }

    fn prepare_job(
        &self,
        domain: Domain,
        pages: Vec<Url>,
        store: &super::plan_store::Store,
        wander_budget: f64,
        total_host_centralities: f64,
    ) -> (Job, DomainStats) {
        let hosts = pages
            .iter()
            .map(|url| Node::from(url).into_host())
            .unique()
            .collect::<Vec<_>>();

        let host_centrality: f64 = hosts
            .iter()
            .map(|host| host.id())
            .map(|id| self.host_centrality.get(&id).unwrap().unwrap_or_default())
            .sum();

        let wander_budget = ((wander_budget * host_centrality) / total_host_centralities).max(1.0);

        let boost = *self.domain_boosts.get(&domain).unwrap_or(&1.0);
        let wander_budget = boost * wander_budget;
        let wander_budget = wander_budget.round() as u64;

        let mut urls: Vec<_> = pages
            .into_iter()
            .unique()
            .map(|url| WeightedUrl {
                url: url.clone(),
                weight: self.config.signals.score(
                    self.page_centrality
                        .get(&Node::from(&url).id())
                        .unwrap()
                        .unwrap_or_default(),
                    store.signal(&url).expect("read planner signal").as_ref(),
                ),
            })
            .collect();

        urls.sort_by(|a, b| a.weight.total_cmp(&b.weight));
        urls.reverse();

        let job = Job {
            domain: domain.clone(),
            urls: urls.into_iter().collect(),
            wandering_urls: wander_budget,
        };

        let domain_stats = DomainStats {
            domain,
            num_hosts: job
                .urls
                .iter()
                .map(|url| url.url.host_str())
                .unique()
                .count(),
            scheduled_urls: job.urls.len() as u64,
            wander_budget,
        };

        (job, domain_stats)
    }

    fn assign_budgets(&self, store: &Arc<super::plan_store::Store>) -> Result<()> {
        let total_crawl_budget = self.config.crawl_budget as f64;
        let num_hosts = self.host_centrality.len();
        let num_hosts = ((num_hosts as f64) * self.config.top_host_fraction).ceil() as usize;

        let crawl_budget_top_hosts_boost = total_crawl_budget
            * self
                .config
                .top_hosts_budget_boost
                .as_ref()
                .map(|top_hosts_budget_boost| top_hosts_budget_boost.reserved_budget_fraction)
                .unwrap_or_default();
        let total_crawl_budget = total_crawl_budget - crawl_budget_top_hosts_boost;

        let mut prev_sum = 0.0;
        let total_host_centrality =
            sorted_centralities(self.host_centrality.iter(), self.config.sort_memory_bytes)
                .map(|(_, centrality)| centrality)
                .take_while(|centrality| {
                    let sum = prev_sum + centrality;
                    prev_sum = sum;
                    // take as long as budget * c_i / S(c_i) > 0.5
                    // as this will include exactly the hosts where
                    // we crawl at least one page.
                    // S(c_i) denotes the sum of all centralities
                    // up to and including c_i
                    2.0 * total_crawl_budget * centrality > sum
                })
                .take(num_hosts)
                .sum::<f64>();

        let top_hosts_total_centrality = self
            .config
            .top_hosts_budget_boost
            .as_ref()
            .map(|top_hosts_budget_boost| {
                sorted_centralities(self.host_centrality.iter(), self.config.sort_memory_bytes)
                    .take(top_hosts_budget_boost.top_hosts)
                    .map(|(_, centrality)| centrality)
                    .sum::<f64>()
            })
            .unwrap_or_default();

        store.initialize_budgets(
            sorted_centralities(self.host_centrality.iter(), self.config.sort_memory_bytes)
                .take(num_hosts)
                .enumerate()
                .filter_map(|(rank, (id, centrality))| {
                    let mut total =
                        ((total_crawl_budget * centrality) / total_host_centrality).ceil() as u64;

                    if let Some(top_hosts_budget_boost) = &self.config.top_hosts_budget_boost {
                        if rank < top_hosts_budget_boost.top_hosts {
                            total += ((crawl_budget_top_hosts_boost * centrality)
                                / top_hosts_total_centrality)
                                .round() as u64;
                        }
                    }

                    let budget = Budget {
                        remaining_schedulable: (total as f64 * (1.0 - self.config.wander_fraction))
                            .ceil() as u64,
                    };

                    if budget.remaining_schedulable > 0 {
                        Some((id.as_u128(), budget.remaining_schedulable))
                    } else {
                        None
                    }
                }),
        )
    }

    async fn pages_to_crawl(
        &self,
        store: Arc<super::plan_store::Store>,
    ) -> Result<(Vec<super::plan_store::Group>, u64)> {
        self.assign_budgets(&store)?;
        let mut grouper = UrlGrouper {
            store: store.clone(),
            harmonic_group: self.harmonic_group.clone(),
            harmonic_rank: self.host_centrality_rank.clone(),
            signal_scope: self
                .config
                .graph_generation
                .as_ref()
                .map(|path| crate::webgraph::signals::Bundle::load(path).map(|bundle| bundle.scope))
                .transpose()?,
        };
        let mut processed = store.checkpoint("pages_processed")?;
        let mut futures = FuturesOrdered::new();

        let num_pages = self.page_centrality.len();
        for chunk in sorted_centralities(self.page_centrality.iter(), self.config.sort_memory_bytes)
            .map(|(id, _)| id)
            .skip(processed as usize)
            .progress_count(num_pages as u64)
            .chunks(128)
            .into_iter()
        {
            while futures.len() >= CONCURRENCY_LIMIT {
                let nodes: Vec<Option<Node>> =
                    futures.next().await.context("planner RPC stream ended")??;
                add_nodes_to_grouper(nodes.into_iter().flatten(), &mut grouper)?;
                processed += 128;
                store.set_checkpoint("pages_processed", processed)?;
            }

            if store.budget_count()? == 0 {
                break;
            }

            let nodes = chunk.collect::<Vec<_>>();
            let webgraph = self.webgraph.clone();

            futures.push_back(async move {
                webgraph
                    .batch_search(nodes.into_iter().map(Id2NodeQuery::Page).collect())
                    .await
            });
        }

        while let Some(nodes) = futures.next().await {
            add_nodes_to_grouper(nodes?.into_iter().flatten(), &mut grouper)?;
            processed += 128;
            store.set_checkpoint("pages_processed", processed)?;
        }

        // make sure frontpage is added for all hosts with a budget.
        // this is necessary to ensure that we crawl at least one page
        // for each host.
        let mut futures = FuturesOrdered::new();
        let num_missing_budgets = store.budget_count()?;
        for chunk in store
            .budget_ids()
            .map(NodeID::from)
            .progress_count(num_missing_budgets as u64)
            .chunks(128)
            .into_iter()
        {
            while futures.len() >= CONCURRENCY_LIMIT {
                let nodes: Vec<Option<Node>> =
                    futures.next().await.context("planner RPC stream ended")??;

                let nodes = nodes.into_iter().flatten().filter_map(|node| {
                    if let Ok(url) = Url::parse(&format!("https://{}", node.as_str())) {
                        Some(Node::from(url))
                    } else {
                        None
                    }
                });

                add_nodes_to_grouper(nodes, &mut grouper)?;
            }

            let nodes = chunk.collect::<Vec<_>>();
            let webgraph = self.webgraph.clone();

            futures.push_back(async move {
                webgraph
                    .batch_search(nodes.into_iter().map(Id2NodeQuery::Host).collect())
                    .await
            });
        }

        while let Some(nodes) = futures.next().await {
            add_nodes_to_grouper(
                nodes?.into_iter().flatten().filter_map(|node| {
                    Url::parse(&format!("https://{}", node.as_str()))
                        .ok()
                        .map(Node::from)
                }),
                &mut grouper,
            )?;
        }

        let missing_budgets = store.remaining_budget()?;
        Ok((grouper.into_groups(), missing_budgets))
    }

    pub async fn build(self) -> Result<()> {
        use std::io::Write;
        let output = Path::new(&self.config.output_path);
        std::fs::create_dir_all(output)?;
        let mut sources = Vec::new();
        for source in [
            &self.config.host_harmonic_path,
            &self.config.page_harmonic_path,
            &self.config.host_centrality_rank_store_path,
        ] {
            sources.push((source, source_digest(Path::new(source))?));
        }
        let signal_hash = self
            .config
            .signals
            .file
            .as_ref()
            .map(|path| {
                use std::io::Read;
                let mut file = std::fs::File::open(path)?;
                let mut hasher = ring::digest::Context::new(&ring::digest::SHA256);
                let mut buffer = [0; 8192];
                loop {
                    let n = file.read(&mut buffer)?;
                    if n == 0 {
                        break;
                    }
                    hasher.update(&buffer[..n]);
                }
                Ok::<_, anyhow::Error>(crate::capture::hex(hasher.finish().as_ref()))
            })
            .transpose()?;
        let signature = crate::capture::sha256(&serde_json::to_vec(&(
            "planner-priority-v2",
            &self.config,
            sources,
            signal_hash,
        ))?);
        let store = super::plan_store::Store::open(&output.join("planner.redb"), &signature)?;
        if store.checkpoint("published")? == 1 {
            return Ok(());
        }
        if let Some(path) = &self.config.signals.file {
            store.import_signals(path)?;
        }
        let (groups, missing_budgets) = if store.checkpoint("pages_complete")? == 1 {
            (
                (0..NUM_GROUPS as u64).map(|id| store.group(id)).collect(),
                store.remaining_budget()?,
            )
        } else {
            let result = self.pages_to_crawl(store.clone()).await?;
            store.set_checkpoint("pages_complete", 1)?;
            result
        };
        let queue_path = output.join("job_queue");
        std::fs::create_dir_all(&queue_path)?;
        let mut queues = (0..self.config.num_job_queues)
            .map(|i| FileQueueWriter::new(queue_path.join(format!("{i}.queue"))))
            .collect::<Result<Vec<FileQueueWriter<Job>>>>()?;
        let mut plan =
            std::io::BufWriter::new(std::fs::File::create(output.join("seeds.jsonl.tmp"))?);
        let mut stats_file =
            std::io::BufWriter::new(std::fs::File::create(output.join("metadata.json.tmp"))?);
        stats_file.write_all(b"{\"schema_version\":2,\"stats\":[")?;
        let mut cursors = vec![None; groups.len()];
        let mut complete = vec![false; groups.len()];
        let mut ordinal = 0u64;
        let mut scheduled = 0u64;
        let mut wandered = 0u64;
        let total_host_centralities = self
            .host_centrality
            .iter()
            .map(|(_, value)| value)
            .sum::<f64>()
            .max(f64::EPSILON);
        let wander_cap = (self.config.crawl_budget as f64 * self.config.wander_fraction).round()
            as u64
            + missing_budgets;
        let hosts = store.host_count()?;
        let domains = store.tenant_count()?;
        let mut first = true;
        let mut last_domain = vec![String::new(); groups.len()];
        loop {
            let high = complete.iter().position(|done| !done);
            let Some(high) = high else { break };
            let index = select_group(&complete, ordinal, self.config.exploration_fraction).unwrap();
            let group = &groups[index];
            let Some((domain, pages, cursor)) = group.next_job(
                &cursors[index],
                self.config.max_urls_per_job,
                self.config.job_bytes,
            )?
            else {
                complete[index] = true;
                continue;
            };
            cursors[index] = Some(cursor);
            if domain.as_str().is_empty() || self.excluded_domains.contains(&domain) {
                continue;
            }

            let wander_budget = self.config.crawl_budget as f64 * self.config.wander_fraction
                + missing_budgets as f64;
            let (mut job, domain_stats) = self.prepare_job(
                domain.clone(),
                pages,
                &store,
                wander_budget,
                total_host_centralities,
            );
            if last_domain[index] == domain.as_str() {
                job.wandering_urls = 0;
            } else {
                last_domain[index] = domain.as_str().to_string();
            }
            job.wandering_urls = job.wandering_urls.min(wander_cap.saturating_sub(wandered));
            for url in &job.urls {
                let seed = serde_json::json!({"schema_version":1,"url":url.url,"weight":url.weight,"tenant":domain.as_str(),"rank_group":index,"source":"webgraph-centrality","discovered_at":chrono::Utc::now(),"configuration_sha256":signature,"exploration":index != high,"signals":store.signal(&url.url)?});
                serde_json::to_writer(&mut plan, &seed)?;
                plan.write_all(b"\n")?;
            }
            if !first {
                stats_file.write_all(b",")?;
            }
            first = false;
            serde_json::to_writer(&mut stats_file, &domain_stats)?;
            scheduled += job.urls.len() as u64;
            wandered += job.wandering_urls;
            let queue =
                bloom::fast_stable_hash_64(domain.as_str().as_bytes()) as usize % queues.len();
            queues[queue].push(job)?;
            ordinal += 1;
        }
        write!(stats_file, "],\"total_scheduled_urls\":{scheduled},\"total_wander_budget\":{wandered},\"total_num_hosts\":{hosts},\"total_num_domains\":{domains}}}")?;
        stats_file.flush()?;
        stats_file.get_ref().sync_all()?;
        plan.flush()?;
        plan.get_ref().sync_all()?;
        for queue in queues {
            queue.finalize()?;
        }
        for i in 0..self.config.num_job_queues {
            std::fs::File::open(queue_path.join(format!("{i}.queue/data")))?.sync_all()?;
        }
        std::fs::rename(output.join("seeds.jsonl.tmp"), output.join("seeds.jsonl"))?;
        std::fs::rename(
            output.join("metadata.json.tmp"),
            output.join("metadata.json"),
        )?;
        std::fs::File::open(output)?.sync_all()?;
        store.set_checkpoint("published", 1)?;
        Ok(())
    }
}

fn add_nodes_to_grouper(nodes: impl Iterator<Item = Node>, grouper: &mut UrlGrouper) -> Result<()> {
    let records = nodes.filter_map(|node| {
        let url = Url::parse(&format!("https://{}", node.as_str())).ok()?;
        Some((
            grouper.group(&url) as u64,
            url,
            node.into_host().id().as_u128(),
        ))
    });
    for batch in records.chunks(128).into_iter() {
        grouper.store.insert_batch(&batch.collect::<Vec<_>>())?;
    }
    Ok(())
}

#[derive(serde::Serialize)]
struct DomainStats {
    domain: Domain,
    num_hosts: usize,
    scheduled_urls: u64,
    wander_budget: u64,
}

impl DomainStats {
    pub fn schedule_budget(&self) -> u64 {
        self.scheduled_urls + self.wander_budget
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_child_file_invalidates_checkpoint_signature() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pages");
        std::fs::write(&file, b"before").unwrap();
        let before = source_digest(dir.path()).unwrap();
        std::fs::write(file, b"after!").unwrap();
        assert_ne!(before, source_digest(dir.path()).unwrap());
    }
    #[test]
    fn schemeless_domain_lookup_preserves_known_rank_and_unknown_tail() {
        let dir = tempfile::tempdir().unwrap();
        let mut rank = speedy_kv::Db::open_or_create(dir.path().join("rank")).unwrap();
        let id = Node::from(Url::parse("https://example.org/").unwrap())
            .into_host()
            .id();
        rank.insert(id, 0).unwrap();
        rank.commit().unwrap();
        let grouper = UrlGrouper {
            store: super::super::plan_store::Store::open(&dir.path().join("plan.redb"), "fixture")
                .unwrap(),
            harmonic_rank: Arc::new(rank),
            harmonic_group: Arc::new(HarmonicRankGroup::new(1000, NUM_GROUPS as u64)),
            signal_scope: None,
        };
        assert_eq!(
            grouper.group(&Url::parse("https://news.example.org/a").unwrap()),
            0
        );
        assert_eq!(
            grouper.group(&Url::parse("https://unknown.test/a").unwrap()),
            NUM_GROUPS - 1
        );
    }
}
