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
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use chrono::Utc;
use itertools::Itertools;
use std::path::Path;
use std::sync::Arc;

use tracing::debug;

pub use super::indexable_webpage::IndexableWebpage;
pub use super::job::{Job, JobSettings};
use crate::backlink_grouper::BacklinkGrouper;
use crate::config::{GossipConfig, IndexerConfig, IndexerDualEncoderConfig};
use crate::distributed::cluster::Cluster;
use crate::models::dual_encoder::DualEncoder as DualEncoderModel;
use crate::webgraph::remote::RemoteWebgraph;
use crate::Result;

use crate::index::Index;
use crate::rake::RakeModel;
use crate::ranking::SignalComputer;
use crate::webgraph::{self, EdgeLimit, Node, NodeID, SmallEdgeWithLabel};
use crate::webpage::{safety_classifier, Html, Webpage};

const MAX_BACKLINKS: EdgeLimit = EdgeLimit::Limit(1024);

#[derive(Clone)]
pub enum IndexerGraphConfig {
    Local { path: String },
    Remote { gossip: GossipConfig },
    Existing { cluster: Arc<Cluster> },
}

impl From<crate::config::IndexerGraphConfig> for IndexerGraphConfig {
    fn from(conf: crate::config::IndexerGraphConfig) -> Self {
        match conf {
            crate::config::IndexerGraphConfig::Local { path } => IndexerGraphConfig::Local { path },
            crate::config::IndexerGraphConfig::Remote { gossip } => {
                IndexerGraphConfig::Remote { gossip }
            }
        }
    }
}

#[derive(Clone)]
pub struct Config {
    pub consumer: crate::warc::stream::Limits,
    pub dedup: crate::crawler::dedup::Config,
    pub host_centrality_store_path: String,
    pub page_centrality_store_path: Option<String>,
    pub page_webgraph: Option<IndexerGraphConfig>,
    pub safety_classifier_path: Option<String>,
    pub dual_encoder: Option<IndexerDualEncoderConfig>,
}

impl From<IndexerConfig> for Config {
    fn from(config: IndexerConfig) -> Self {
        Self {
            dedup: config.dedup,
            consumer: config.consumer,
            host_centrality_store_path: config.host_centrality_store_path,
            page_centrality_store_path: config.page_centrality_store_path,
            page_webgraph: config.page_webgraph.map(IndexerGraphConfig::from),
            safety_classifier_path: config.safety_classifier_path,
            dual_encoder: config.dual_encoder,
        }
    }
}

struct DualEncoder {
    model: DualEncoderModel,
    page_centrality_rank_threshold: Option<u64>,
}

pub(super) enum Webgraph {
    Remote(RemoteWebgraph),
    Local(webgraph::Webgraph),
}

impl Webgraph {
    async fn batch_raw_ingoing_edges_with_labels(
        &self,
        ids: Vec<NodeID>,
        limit: EdgeLimit,
    ) -> Vec<Vec<SmallEdgeWithLabel>> {
        let edges = match self {
            Self::Remote(webgraph) => webgraph
                .batch_search(
                    ids.into_iter()
                        .map(|id| {
                            webgraph::query::BacklinksWithLabelsQuery::new(id).with_limit(limit)
                        })
                        .collect(),
                )
                .await
                .unwrap_or_default(),
            Self::Local(webgraph) => {
                let mut res = Vec::new();

                for id in ids {
                    res.push(
                        webgraph
                            .search(
                                &webgraph::query::BacklinksWithLabelsQuery::new(id)
                                    .with_limit(limit),
                            )
                            .unwrap_or_default(),
                    );
                }

                res
            }
        };

        edges.into_iter().map(useful_backlinks).collect()
    }
}

fn useful_backlinks(edges: Vec<SmallEdgeWithLabel>) -> Vec<SmallEdgeWithLabel> {
    edges
        .into_iter()
        .filter(|e| !e.label.is_empty())
        .filter(|e| {
            let label = e.label.to_lowercase();
            let stopwords = [
                "click",
                "click here",
                "here",
                "link",
                "website",
                "webpage",
                "page",
                "site",
                "url",
                "web",
                "visit",
                "more",
                "info",
                "information",
                "read",
                "read more",
            ];

            !stopwords.contains(&label.as_str())
        })
        .collect::<Vec<_>>()
}

impl Webgraph {
    async fn new(config: &IndexerGraphConfig, strict: bool) -> Result<Self> {
        Ok(match config {
            IndexerGraphConfig::Local { path } => {
                Self::Local(webgraph::WebgraphBuilder::new(path, 0u64.into()).open()?)
            }
            IndexerGraphConfig::Remote { gossip } => {
                let cluster = crate::start_gossip_cluster_thread(gossip.clone(), None);
                let remote = RemoteWebgraph::new(cluster).await;
                #[cfg(not(feature = "dev"))]
                {
                    if strict {
                        tokio::time::timeout(
                            std::time::Duration::from_secs(20),
                            remote.await_ready(),
                        )
                        .await
                        .map_err(|_| anyhow::anyhow!("webgraph readiness timeout"))?;
                    } else {
                        remote.await_ready().await;
                    }
                }
                Self::Remote(remote)
            }
            IndexerGraphConfig::Existing { cluster } => {
                let remote = RemoteWebgraph::new(cluster.clone()).await;
                #[cfg(not(feature = "dev"))]
                {
                    if strict {
                        tokio::time::timeout(
                            std::time::Duration::from_secs(20),
                            remote.await_ready(),
                        )
                        .await
                        .map_err(|_| anyhow::anyhow!("webgraph readiness timeout"))?;
                    } else {
                        remote.await_ready().await;
                    }
                }
                Self::Remote(remote)
            }
        })
    }
}

pub struct IndexingWorker {
    pub(super) consumer: crate::warc::stream::Limits,
    host_centrality_store: speedy_kv::Db<NodeID, f64>,
    host_centrality_rank_store: speedy_kv::Db<NodeID, u64>,
    page_centrality_store: Option<speedy_kv::Db<NodeID, f64>>,
    page_centrality_rank_store: Option<speedy_kv::Db<NodeID, u64>>,
    page_webgraph: Option<Webgraph>,
    safety_classifier: Option<safety_classifier::Model>,
    job_settings: Option<JobSettings>,
    rake: RakeModel,
    dual_encoder: Option<DualEncoder>,
    pub(super) dedup: crate::crawler::dedup::Config,
}

impl IndexingWorker {
    pub async fn new(config: Config) -> Self {
        Self::open(config, false)
            .await
            .expect("indexing worker configuration failed")
    }

    pub async fn try_new(config: Config) -> Result<Self> {
        Self::open(config, true).await
    }

    async fn open(config: Config, strict: bool) -> Result<Self> {
        let host_centrality_rank_store = speedy_kv::Db::open_or_create(
            Path::new(&config.host_centrality_store_path).join("harmonic_rank"),
        )?;

        Ok(Self {
            host_centrality_store: speedy_kv::Db::open_or_create(
                Path::new(&config.host_centrality_store_path).join("harmonic"),
            )?,
            host_centrality_rank_store,
            page_centrality_store: config
                .page_centrality_store_path
                .as_ref()
                .map(|p| speedy_kv::Db::open_or_create(Path::new(&p).join("harmonic")))
                .transpose()?,
            page_centrality_rank_store: config
                .page_centrality_store_path
                .as_ref()
                .map(|p| speedy_kv::Db::open_or_create(Path::new(&p).join("harmonic_rank")))
                .transpose()?,
            page_webgraph: match config.page_webgraph.as_ref() {
                Some(graph) => Some(Webgraph::new(graph, strict).await?),
                None => None,
            },
            safety_classifier: config
                .safety_classifier_path
                .as_ref()
                .map(|path| safety_classifier::Model::open(path))
                .transpose()?,
            job_settings: None,
            rake: RakeModel::default(),
            dual_encoder: config
                .dual_encoder
                .as_ref()
                .map(|dual_encoder| {
                    let model = DualEncoderModel::open(&dual_encoder.model_path)?;

                    Ok::<_, anyhow::Error>(DualEncoder {
                        model,
                        page_centrality_rank_threshold: dual_encoder.page_centrality_rank_threshold,
                    })
                })
                .transpose()?,
            dedup: config.dedup,
            consumer: config.consumer,
        })
    }

    pub(super) fn page_centrality_store(&self) -> Option<&speedy_kv::Db<NodeID, f64>> {
        self.page_centrality_store.as_ref()
    }

    pub(super) fn page_webgraph(&self) -> Option<&Webgraph> {
        self.page_webgraph.as_ref()
    }

    pub fn process(&mut self, job: &Job) -> Index {
        job.process(self)
    }

    pub fn set_job_settings(&mut self, job_settings: JobSettings) {
        self.job_settings = Some(job_settings);
    }

    fn prepare(&self, page: &IndexableWebpage) -> Result<Webpage> {
        let _span = super::metrics::Span::new(1);
        let html = match Html::parse_without_text(&page.body, &page.url) {
            Ok(html) => html,
            Err(err) => {
                return Err(anyhow::anyhow!("error parsing html: {:?}", err));
            }
        };

        if html.is_no_index() {
            return Err(anyhow::anyhow!("noindex"));
        }

        let title = html.title().unwrap_or_default();
        if title.is_empty() || title.chars().all(|c| c.is_whitespace()) {
            return Err(anyhow::anyhow!("empty title"));
        }

        Ok(Webpage::from(html))
    }

    fn set_host_centrality(&self, page: &mut Webpage) -> Result<()> {
        let node = Node::from(page.html.url());
        let host_node_id = node.clone().into_host().id();

        let host_centrality = self
            .host_centrality_store
            .get(&host_node_id)
            .unwrap()
            .unwrap_or_default();

        let host_centrality_rank = self
            .host_centrality_rank_store
            .get(&host_node_id)
            .unwrap()
            .unwrap_or(u64::MAX);

        if let Some(host_centrality_threshold) =
            self.job_settings.and_then(|s| s.host_centrality_threshold)
        {
            if host_centrality < host_centrality_threshold {
                return Err(anyhow::anyhow!("low host_centrality value"));
            }
        }

        page.node_id = Some(host_node_id);

        page.host_centrality = host_centrality;
        page.host_centrality_rank = host_centrality_rank;

        if !page.host_centrality.is_finite() {
            page.host_centrality = 0.0;
        }

        self.parse_text(page)?;

        Ok(())
    }

    fn parse_text(&self, page: &mut Webpage) -> Result<()> {
        let _span = super::metrics::Span::new(2);
        page.html.parse_text();

        if page.html.empty_all_text() {
            return Err(anyhow::anyhow!("empty all text"));
        }

        if let Some(minimum_clean_words) = self.job_settings.and_then(|s| s.minimum_clean_words) {
            match page.html.clean_text() {
                Some(clean_text) => {
                    if clean_text.split_whitespace().count() < minimum_clean_words {
                        return Err(anyhow::anyhow!("too few clean words"));
                    }
                }
                None => {
                    return Err(anyhow::anyhow!("no clean text"));
                }
            }
        }

        Ok(())
    }

    fn set_page_centralities(&self, page: &mut Webpage) {
        let node = Node::from(page.html.url());

        if let Some(store) = self.page_centrality_store.as_ref() {
            let node_id = node.id();

            page.page_centrality = store.get(&node_id).unwrap().unwrap_or_default();
        }

        page.page_centrality_rank = u64::MAX;

        if let Some(store) = self.page_centrality_rank_store.as_ref() {
            let node_id = node.id();

            page.page_centrality_rank = store.get(&node_id).unwrap().unwrap_or(u64::MAX);
        }

        if !page.page_centrality.is_finite() {
            page.page_centrality = 0.0;
        }
    }

    fn set_keywords(&self, page: &mut Webpage) {
        page.keywords = page.html.keywords(&self.rake);
    }

    fn set_safety_classification(&self, page: &mut Webpage) {
        if let Some(model) = self.safety_classifier.as_ref() {
            page.safety_classification = Some(model.predict(page).label);
        }
    }

    pub fn set_title_embeddings(&self, pages: &mut [Webpage]) {
        if let Some(dual_encoder) = self.dual_encoder.as_ref() {
            let (page_indexes, titles): (Vec<_>, Vec<_>) = pages
                .iter()
                .enumerate()
                .filter(|(_, w)| {
                    dual_encoder
                        .page_centrality_rank_threshold
                        .map(|thresh| w.page_centrality_rank <= thresh)
                        .unwrap_or(true)
                })
                .map(|(i, w)| (i, w.html.title().unwrap_or_default()))
                .unzip();

            let title_emb = dual_encoder
                .model
                .embed(&titles)
                .ok()
                .and_then(|t| t.to_dtype(candle_core::DType::BF16).ok());

            if let Some(title_emb) = title_emb {
                for (i, page_index) in page_indexes.into_iter().enumerate() {
                    if let Ok(emb) = title_emb.get(i) {
                        pages[page_index].title_embedding = Some(emb);
                    }
                }
            }
        }
    }

    pub async fn set_backlinks(&self, pages: &mut [Webpage]) {
        if let Some(graph) = self.page_webgraph() {
            let ids = pages
                .iter()
                .map(|w| Node::from(w.html.url()).id())
                .collect::<Vec<_>>();

            let backlinks = graph
                .batch_raw_ingoing_edges_with_labels(ids, MAX_BACKLINKS)
                .await;

            for (page, backlinks) in pages.iter_mut().zip_eq(backlinks) {
                page.set_backlinks(backlinks.clone());

                let mut grouper =
                    BacklinkGrouper::new(self.host_centrality_rank_store.len() as u64);

                for backlink in backlinks {
                    let from = self
                        .host_centrality_rank_store
                        .get(&backlink.from)
                        .unwrap()
                        .unwrap_or(u64::MAX);

                    grouper.add(backlink, from);
                }

                page.set_grouped_backlinks(grouper.groups())
            }
        }
    }

    pub fn set_keyword_embeddings(&self, pages: &mut [Webpage]) {
        if let Some(dual_encoder) = self.dual_encoder.as_ref() {
            let (page_indexes, keywords): (Vec<_>, Vec<_>) = pages
                .iter()
                .enumerate()
                .filter(|(_, w)| {
                    dual_encoder
                        .page_centrality_rank_threshold
                        .map(|thresh| w.page_centrality_rank <= thresh)
                        .unwrap_or(true)
                })
                .map(|(i, w)| (i, w.keywords.join("\n")))
                .unzip();

            let keyword_emb = dual_encoder
                .model
                .embed(&keywords)
                .ok()
                .and_then(|t| t.to_dtype(candle_core::DType::BF16).ok());

            if let Some(keyword_emb) = keyword_emb {
                for (i, page_index) in page_indexes.into_iter().enumerate() {
                    if let Ok(emb) = keyword_emb.get(i) {
                        pages[page_index].keyword_embedding = Some(emb);
                    }
                }
            }
        }
    }

    pub async fn prepare_one(
        &self,
        page: &IndexableWebpage,
        cache: &mut TextCache,
    ) -> Result<Webpage> {
        self.prepare_one_with_text(page, cache, None).await
    }

    pub async fn prepare_one_with_text(
        &self,
        page: &IndexableWebpage,
        cache: &mut TextCache,
        text: Option<String>,
    ) -> Result<Webpage> {
        let mut webpage = self.prepare(page)?;
        webpage.html.preserve_url_identity(&page.url)?;
        webpage.fetch_time_ms = page.fetch_time_ms;
        {
            let _span = super::metrics::Span::new(3);
            let host = Node::from(webpage.html.url()).into_host().id();
            webpage.node_id = Some(host);
            webpage.host_centrality = self.host_centrality_store.get(&host)?.unwrap_or_default();
            if !webpage.host_centrality.is_finite() {
                webpage.host_centrality = 0.;
            }
            webpage.host_centrality_rank = self
                .host_centrality_rank_store
                .get(&host)?
                .unwrap_or(u64::MAX);
            if self
                .job_settings
                .and_then(|s| s.host_centrality_threshold)
                .is_some_and(|min| webpage.host_centrality < min)
            {
                anyhow::bail!("low host_centrality value");
            }
            if let Some(store) = &self.page_centrality_store {
                webpage.page_centrality = store
                    .get(&Node::from(webpage.html.url()).id())?
                    .unwrap_or_default();
            }
            webpage.page_centrality_rank = if let Some(store) = &self.page_centrality_rank_store {
                store
                    .get(&Node::from(webpage.html.url()).id())?
                    .unwrap_or(u64::MAX)
            } else {
                u64::MAX
            };
        }
        {
            let _span = super::metrics::Span::new(2);
            cache.extract(&page.body, &mut webpage.html);
            if let Some(text) = text {
                webpage.html.set_clean_text(text);
            }
            anyhow::ensure!(!webpage.html.empty_all_text(), "empty all text");
            if let Some(minimum) = self.job_settings.and_then(|s| s.minimum_clean_words) {
                anyhow::ensure!(
                    webpage
                        .html
                        .clean_text()
                        .is_some_and(|s| s.split_whitespace().count() >= minimum),
                    "too few clean words"
                );
            }
        }
        {
            let _span = super::metrics::Span::new(3);
            self.set_keywords(&mut webpage);
            self.set_safety_classification(&mut webpage);
            let mut signal = SignalComputer::new(None);
            signal.set_current_timestamp(Utc::now().timestamp().max(0) as usize);
            webpage.pre_computed_score = signal.precompute_score(&webpage);
            if let Some(encoder) = &self.dual_encoder {
                if encoder
                    .page_centrality_rank_threshold
                    .is_none_or(|max| webpage.page_centrality_rank <= max)
                {
                    let title = webpage.html.title().unwrap_or_default();
                    webpage.title_embedding = Some(
                        encoder
                            .model
                            .embed(&[title])?
                            .to_dtype(candle_core::DType::BF16)?
                            .get(0)?,
                    );
                    let keywords = webpage.keywords.join("\n");
                    webpage.keyword_embedding = Some(
                        encoder
                            .model
                            .embed(&[keywords])?
                            .to_dtype(candle_core::DType::BF16)?
                            .get(0)?,
                    );
                }
            }
            if let Some(graph) = self.page_webgraph() {
                let query = webgraph::query::BacklinksWithLabelsQuery::new(
                    Node::from(webpage.html.url()).id(),
                )
                .with_limit(MAX_BACKLINKS);
                let edges = match graph {
                    Webgraph::Local(graph) => graph.search(&query)?,
                    Webgraph::Remote(graph) => tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        graph.batch_search(vec![query]),
                    )
                    .await
                    .map_err(|_| anyhow::anyhow!("webgraph query timeout"))??
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("missing webgraph response"))?,
                };
                let backlinks = useful_backlinks(edges);
                webpage.set_backlinks(backlinks.clone());
                let mut grouper =
                    BacklinkGrouper::new(self.host_centrality_rank_store.len() as u64);
                for backlink in backlinks {
                    let from = self
                        .host_centrality_rank_store
                        .get(&backlink.from)?
                        .unwrap_or(u64::MAX);
                    grouper.add(backlink, from);
                }
                webpage.set_grouped_backlinks(grouper.groups());
            }
        }
        Ok(webpage)
    }

    pub async fn prepare_webpages(&self, batch: &[IndexableWebpage]) -> Vec<Webpage> {
        self.prepare_webpages_with_positions(batch)
            .await
            .into_iter()
            .map(|(_, page)| page)
            .collect()
    }

    pub(crate) async fn prepare_webpages_with_positions(
        &self,
        batch: &[IndexableWebpage],
    ) -> Vec<(usize, Webpage)> {
        let mut positions = Vec::with_capacity(batch.len());
        let mut res = Vec::with_capacity(batch.len());
        let mut signal_computer = SignalComputer::new(None);

        for (position, page) in batch.iter().enumerate() {
            let mut webpage = match self.prepare(page) {
                Ok(html) => html,
                Err(err) => {
                    debug!("skipping webpage: {}", err);
                    continue;
                }
            };
            if let Err(e) = self.set_host_centrality(&mut webpage) {
                debug!("skipping webpage: {}", e);
                continue;
            }

            let signal_span = super::metrics::Span::new(3);
            self.set_page_centralities(&mut webpage);
            self.set_keywords(&mut webpage);
            self.set_safety_classification(&mut webpage);

            signal_computer.set_current_timestamp(Utc::now().timestamp().max(0) as usize);
            webpage.pre_computed_score = signal_computer.precompute_score(&webpage);

            drop(signal_span);
            positions.push(position);
            res.push(webpage);
        }

        self.set_title_embeddings(&mut res);
        self.set_keyword_embeddings(&mut res);
        self.set_backlinks(&mut res).await;

        positions.into_iter().zip(res).collect()
    }
}

#[cfg(test)]
mod tests {
    use file_store::temp::TempDir;

    use crate::config::WarcSource;

    use super::*;

    fn setup_worker(data_path: &Path, threshold: Option<u64>) -> (IndexingWorker, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let worker = crate::block_on(IndexingWorker::new(
            IndexerConfig {
                consumer: Default::default(),
                dedup: Default::default(),
                host_centrality_store_path: temp_dir
                    .as_ref()
                    .join("host_centrality")
                    .to_str()
                    .unwrap()
                    .to_string(),
                page_centrality_store_path: None,
                page_webgraph: None,
                safety_classifier_path: None,
                dual_encoder: Some(IndexerDualEncoderConfig {
                    model_path: data_path.to_str().unwrap().to_string(),
                    page_centrality_rank_threshold: threshold,
                }),
                output_path: temp_dir
                    .as_ref()
                    .join("output")
                    .to_str()
                    .unwrap()
                    .to_string(),
                limit_warc_files: None,
                skip_warc_files: None,
                warc_source: WarcSource::Local(crate::config::LocalConfig {
                    folder: temp_dir.as_ref().join("warc").to_str().unwrap().to_string(),
                    names: vec!["".to_string()],
                }),
                host_centrality_threshold: None,
                minimum_clean_words: None,
                batch_size: 10,
                autocommit_after_num_inserts:
                    crate::config::defaults::Indexing::autocommit_after_num_inserts(),
            }
            .into(),
        ));

        (worker, temp_dir)
    }

    #[test]
    fn title_embeddings() {
        let data_path = Path::new("../../data/summarizer/dual_encoder");
        if !data_path.exists() {
            // Skip the test if the test data is not available
            return;
        }
        let (worker, _temp_dir) = setup_worker(data_path, None);

        let webpages = vec![
            IndexableWebpage {
                url: "https://a.com".to_string(),
                body: "<html><head><title>Homemade Heart Brownie Recipe</title></head><body>Example</body></html>"
                    .to_string(),
                fetch_time_ms: 0,
            },
            IndexableWebpage {
                url: "https://b.com".to_string(),
                body: "<html><head><title>How To Use an iMac as a Monitor for a PC</title></head><body>Example</body></html>"
                    .to_string(),
                fetch_time_ms: 0,
            },
        ];

        let webpages = crate::block_on(worker.prepare_webpages(&webpages));

        assert_eq!(webpages.len(), 2);

        assert_eq!(
            webpages[0].html.title(),
            Some("Homemade Heart Brownie Recipe".to_string())
        );
        assert_eq!(
            webpages[1].html.title(),
            Some("How To Use an iMac as a Monitor for a PC".to_string())
        );

        assert!(webpages.iter().all(|w| w.title_embedding.is_some()));

        let emb1 = webpages[0].title_embedding.as_ref().unwrap();
        let emb2 = webpages[1].title_embedding.as_ref().unwrap();

        let query = "best chocolate cake";
        let query_emb = worker
            .dual_encoder
            .as_ref()
            .unwrap()
            .model
            .embed(&[query.to_string()])
            .unwrap()
            .to_dtype(candle_core::DType::F16)
            .unwrap()
            .get(0)
            .unwrap();

        let sim1: f32 = query_emb
            .unsqueeze(0)
            .unwrap()
            .matmul(
                &emb1
                    .to_dtype(candle_core::DType::F16)
                    .unwrap()
                    .unsqueeze(0)
                    .unwrap()
                    .t()
                    .unwrap(),
            )
            .unwrap()
            .get(0)
            .unwrap()
            .squeeze(0)
            .unwrap()
            .to_dtype(candle_core::DType::F32)
            .unwrap()
            .to_vec0()
            .unwrap();

        let sim2: f32 = query_emb
            .unsqueeze(0)
            .unwrap()
            .matmul(
                &emb2
                    .to_dtype(candle_core::DType::F16)
                    .unwrap()
                    .unsqueeze(0)
                    .unwrap()
                    .t()
                    .unwrap(),
            )
            .unwrap()
            .get(0)
            .unwrap()
            .squeeze(0)
            .unwrap()
            .to_dtype(candle_core::DType::F32)
            .unwrap()
            .to_vec0()
            .unwrap();

        assert!(sim1 > sim2);
    }

    #[test]
    fn title_embedding_ranks() {
        let data_path = Path::new("../../data/summarizer/dual_encoder");
        if !data_path.exists() {
            // Skip the test if the test data is not available
            return;
        }
        let (worker, _temp_dir) = setup_worker(data_path, Some(100_000));

        let mut a = Webpage::test_parse("<html><head><title>Homemade Heart Brownie Recipe</title></head><body>Example</body></html>", "https://a.com").unwrap();
        a.page_centrality_rank = 1;

        let mut b = Webpage::test_parse("<html><head><title>How To Use an iMac as a Monitor for a PC</title></head><body>Example</body></html>", "https://b.com").unwrap();
        b.page_centrality_rank = 1_000_000;

        let mut webpages = vec![a, b];
        worker.set_title_embeddings(&mut webpages);

        assert_eq!(webpages.len(), 2);
        assert_eq!(
            webpages[0].html.title(),
            Some("Homemade Heart Brownie Recipe".to_string())
        );

        assert!(webpages[0].title_embedding.is_some());
        assert!(webpages[1].title_embedding.is_none());

        let mut a = Webpage::test_parse("<html><head><title>Homemade Heart Brownie Recipe</title></head><body>Example</body></html>", "https://a.com").unwrap();
        a.page_centrality_rank = 1_000_000;

        let mut b = Webpage::test_parse("<html><head><title>How To Use an iMac as a Monitor for a PC</title></head><body>Example</body></html>", "https://b.com").unwrap();
        b.page_centrality_rank = 1;

        let mut webpages = vec![a, b];
        worker.set_title_embeddings(&mut webpages);

        assert_eq!(webpages.len(), 2);
        assert_eq!(
            webpages[0].html.title(),
            Some("Homemade Heart Brownie Recipe".to_string())
        );

        assert!(webpages[0].title_embedding.is_none());
        assert!(webpages[1].title_embedding.is_some());
    }
}

pub struct TextCache {
    entries: std::collections::VecDeque<(
        String,
        (Option<String>, Option<String>, Option<whatlang::Lang>),
        usize,
    )>,
    bytes: usize,
    limit: usize,
    pub hits: u64,
}
impl TextCache {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Default::default(),
            bytes: 0,
            limit,
            hits: 0,
        }
    }
    fn extract(&mut self, body: &str, html: &mut Html) {
        let key = crate::capture::sha256(body.as_bytes());
        if let Some((_, text, _)) = self
            .entries
            .iter()
            .find(|(candidate, _, _)| candidate == &key)
        {
            html.restore_text(text);
            self.hits += 1;
            return;
        }
        html.parse_text();
        if self.limit == 0 {
            return;
        }
        let text = html.text_snapshot();
        let bytes = text.0.as_ref().map_or(0, String::capacity)
            + text.1.as_ref().map_or(0, String::capacity)
            + key.capacity()
            + 128;
        if bytes > self.limit {
            return;
        }
        while self.bytes + bytes > self.limit || self.entries.len() >= 64 {
            if let Some((_, _, previous)) = self.entries.pop_front() {
                self.bytes -= previous;
            } else {
                break;
            }
        }
        self.entries.push_back((key, text, bytes));
        self.bytes += bytes;
    }
}
