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

use crate::{
    canon_index::CanonicalIndex,
    config::{self, WarcSource, WebgraphConstructConfig},
    webgraph::{self, Edge, Node, NodeID},
    webpage::Html,
    Result,
};
use itertools::Itertools;
use url::Url;

use std::{path::Path, sync::Arc};
use tracing::{info, trace};

#[derive(Debug, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode, Clone)]
pub enum JobConfig {
    Http(config::HttpConfig),
    Local(config::LocalConfig),
    S3(config::S3Config),
}

impl From<config::WarcSource> for JobConfig {
    fn from(value: config::WarcSource) -> Self {
        match value {
            config::WarcSource::HTTP(config) => JobConfig::Http(config),
            config::WarcSource::Local(config) => JobConfig::Local(config),
            config::WarcSource::S3(config) => JobConfig::S3(config),
        }
    }
}

impl From<JobConfig> for config::WarcSource {
    fn from(value: JobConfig) -> Self {
        match value {
            JobConfig::Http(config) => config::WarcSource::HTTP(config),
            JobConfig::Local(config) => config::WarcSource::Local(config),
            JobConfig::S3(config) => WarcSource::S3(config),
        }
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode, Clone)]
pub struct Job {
    pub config: JobConfig,
    pub warc_paths: Vec<String>,
}

fn canonical_or_self(index: &CanonicalIndex, url: Url) -> Url {
    if let Some(url) = index.get(&url).unwrap() {
        url
    } else {
        url
    }
}

pub struct WebgraphWorker {
    pub host_centrality_store: Option<Arc<speedy_kv::Db<NodeID, f64>>>,
    pub host_rank_store: Option<Arc<speedy_kv::Db<NodeID, u64>>>,
    pub graph: webgraph::Webgraph,
    pub canonical_index: Option<Arc<CanonicalIndex>>,
}

impl WebgraphWorker {
    pub fn process_job(&mut self, job: &Job) {
        self.try_process_job(job, crate::warc::stream::Limits::default())
            .expect("webgraph consumer failed; input retained");
    }

    pub fn try_process_job(
        &mut self,
        job: &Job,
        limits: crate::warc::stream::Limits,
    ) -> Result<()> {
        limits.validate()?;
        self.graph.prepare_writer_with_budget(limits.batch_bytes)?;
        let name = job.warc_paths.first().unwrap().split('/').last().unwrap();

        info!("processing {}", name);

        let source = WarcSource::from(job.config.clone());

        for path in &job.warc_paths {
            let mut consumer = crate::warc::consumer::Consumer::open(
                &source,
                path,
                &self.graph.path().join("consumer-checkpoints"),
                limits,
            )?;
            consumer.recover_commit(self.graph.commit_payload()?.as_deref())?;
            loop {
                let batch={let _span=crate::webgraph::metrics::Span::new(0);consumer.next_batch(64)?};
                let Some(batch)=batch else {break;};
                for record in batch.records {
                    if !record.links_allowed() || record.is_revalidation() {
                        continue;
                    }
                    let webpage = match {let _span=crate::webgraph::metrics::Span::new(1);Html::parse_without_text(
                        &record.response.body,
                        &record.request.url,
                    )} {
                        Ok(webpage) => webpage,
                        Err(err) => {
                            tracing::error!("error parsing webpage: {}", err);
                            continue;
                        }
                    };

                    if webpage.is_no_follow() {
                        continue;
                    }
                    let mut source = webpage.url().clone();
                    if let Some(index) = &self.canonical_index {
                        source = canonical_or_self(index, source);
                    }

                    let source = Node::from(source);
                    let source_centrality = self
                        .host_centrality_store
                        .as_ref()
                        .and_then(|store| store.get(&source.clone().into_host().id()).unwrap())
                        .unwrap_or(0.0);
                    let source_rank = self
                        .host_rank_store
                        .as_ref()
                        .and_then(|store| store.get(&source.clone().into_host().id()).unwrap())
                        .unwrap_or(u64::MAX);

                    let _links_span=crate::webgraph::metrics::Span::new(2);
                    let num_outgoing_hosts_from_page = webpage
                        .anchor_links()
                        .into_iter()
                        .filter_map(|l| l.destination.host_str().map(|h| h.to_string()))
                        .unique()
                        .count() as u64;

                    for mut link in webpage.anchor_links().into_iter() {
                        let mut destination = link.destination.clone();

                        if let Some(index) = &self.canonical_index {
                            destination = canonical_or_self(index, destination);
                        }

                        link.text = link.text.chars().take(128).collect();

                        let destination = Node::from(destination);

                        let destination_centrality = self
                            .host_centrality_store
                            .as_ref()
                            .and_then(|store| {
                                store.get(&destination.clone().into_host().id()).unwrap()
                            })
                            .unwrap_or(0.0);
                        let destination_rank = self
                            .host_rank_store
                            .as_ref()
                            .and_then(|store| {
                                store.get(&destination.clone().into_host().id()).unwrap()
                            })
                            .unwrap_or(u64::MAX);

                        trace!("inserting link {:?}", link);
                        self.graph
                            .insert(Edge {
                                from: source.clone(),
                                to: destination,
                                rel_flags: link.rel,
                                label: link.text,
                                sort_score: source_rank.saturating_add(destination_rank),
                                from_centrality: source_centrality,
                                to_centrality: destination_centrality,
                                from_rank: source_rank,
                                to_rank: destination_rank,
                                num_outgoing_hosts_from_page,
                            })
                            .unwrap();
                    }
                }

                let payload = consumer.payload(batch.next_record, false, Vec::new())?;
                let _span=crate::webgraph::metrics::Span::new(5);
                self.graph.commit_with_payload(&payload)?;
                consumer.confirm(batch.next_record, false)?;
            }
            let payload = consumer.payload(consumer.cursor(), true, Vec::new())?;
            self.graph.commit_with_payload(&payload)?;
            consumer.finish()?;
        }

        info!("{} done", name);
        Ok(())
    }
}

pub struct Webgraph {}

impl Webgraph {
    pub fn run(config: &WebgraphConstructConfig) -> Result<()> {
        use rayon::prelude::*;
        config.consumer.validate()?;
        anyhow::ensure!(
            config.limit_warc_files != Some(0),
            "webgraph file limit must be positive"
        );
        let workspace =
            crate::warc::publication::Workspace::open(Path::new(&config.graph_base_path))?;
        let paths = config
            .warc_source
            .paths()?
            .into_iter()
            .unique()
            .skip(config.skip_warc_files.unwrap_or(0))
            .take(config.limit_warc_files.unwrap_or(usize::MAX))
            .collect::<Vec<_>>();
        anyhow::ensure!(
            !paths.is_empty(),
            "no WARC inputs selected for webgraph publication"
        );
        let canonical_index = config
            .canonical_index_path
            .as_ref()
            .map(|path| CanonicalIndex::open(path).map(Arc::new))
            .transpose()?;
        let host_centrality_store = Arc::new(speedy_kv::Db::open_or_create(
            &config.host_centrality_store_path,
        )?);
        let host_rank_store =
            Arc::new(speedy_kv::Db::open_or_create(&config.host_rank_store_path)?);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(
                config
                    .consumer
                    .workers
                    .min(crate::crawler::resources::effective_cpus()),
            )
            .stack_size(config.consumer.stack_bytes)
            .build()?;
        let inputs: Result<Vec<_>> = pool.install(|| {
            paths
                .into_par_iter()
                .map(|path| {
                    let key = crate::warc::publication::Workspace::key(&config.warc_source, &path)?;
                    let location = workspace.jobs.join(key);
                    let mut worker = WebgraphWorker {
                        graph: webgraph::Webgraph::open(&location, config.shard)?,
                        host_centrality_store: Some(host_centrality_store.clone()),
                        host_rank_store: Some(host_rank_store.clone()),
                        canonical_index: canonical_index.clone(),
                    };
                    worker.try_process_job(
                        &Job {
                            config: config.warc_source.clone().into(),
                            warc_paths: vec![path],
                        },
                        config.consumer,
                    )?;
                    worker.graph.optimize_read()?;
                    Ok(location)
                })
                .collect()
        });
        let _merge_span=crate::webgraph::metrics::Span::new(6);
        let copies = workspace.copies(&inputs?)?;
        let mut copies = copies.into_iter();
        let first = copies
            .next()
            .ok_or_else(|| anyhow::anyhow!("no completed graph inputs"))?;
        let mut graph = webgraph::Webgraph::open(&first, config.shard)?;
        graph.prepare_writer_with_budget(config.consumer.batch_bytes)?;
        for path in copies {
            let mut other = webgraph::Webgraph::open(path, config.shard)?;
            other.prepare_writer_with_budget(config.consumer.batch_bytes)?;
            graph.merge(other)?;
        }
        if config.merge_all_segments {
            graph.optimize_read()?;
        }
        let location = graph.path();
        drop(graph);
        workspace.publish(&location)?;
        Ok(())
    }
}

#[cfg(test)]
mod capture_contract_tests {
    #[test]
    fn header_nofollow_excludes_edges_but_noindex_alone_keeps_them() {
        for (directive, allowed) in [
            ("noindex", true),
            ("nofollow", false),
            ("otherbot: none", true),
        ] {
            let url = url::Url::parse("https://example.org/page").unwrap();
            let capture = crate::capture::fixture(
                &url,
                uuid::Uuid::new_v4().to_string(),
                b"<html><body><a href='https://other.org/'>link</a></body></html>".to_vec(),
                vec![crate::capture::Header {
                    name: "x-robots-tag".into(),
                    value_hex: crate::capture::hex(directive.as_bytes()),
                }],
            );
            let mut bytes = Vec::new();
            crate::warc::capture::write(
                &mut bytes,
                &capture,
                crate::warc::PayloadType::Html,
                3,
                true,
            )
            .unwrap();
            let record = crate::warc::WarcFile::new(bytes)
                .records()
                .next()
                .unwrap()
                .unwrap();
            assert_eq!(record.links_allowed(), allowed);
        }
    }
}
