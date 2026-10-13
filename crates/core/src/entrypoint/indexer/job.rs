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

use std::path::Path;

use itertools::Itertools;

use tokio::pin;
use tracing::{info, trace, warn};

use crate::config;
use crate::entrypoint::download_all_warc_files;
use crate::index::Index;
use crate::warc::PayloadType;

use super::{IndexableWebpage, IndexingWorker};

#[derive(Debug, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode)]
pub struct Job {
    pub source_config: config::WarcSource,
    pub warc_path: String,
    pub base_path: String,
    pub settings: JobSettings,
}

#[derive(
    Debug, Clone, Copy, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode,
)]
pub struct JobSettings {
    pub host_centrality_threshold: Option<f64>,
    pub minimum_clean_words: Option<usize>,
    pub batch_size: usize,
    pub autocommit_after_num_inserts: usize,
}

impl Job {
    pub fn process(&self, worker: &IndexingWorker) -> Index {
        let name = self.warc_path.split('/').last().unwrap();

        let mut has_host_centrality = false;
        let mut has_page_centrality = false;
        let mut has_backlinks = false;

        info!("processing {}", name);

        let mut index = Index::open(Path::new(&self.base_path).join(name)).unwrap();
        index.prepare_writer().unwrap();

        let mut exact = crate::crawler::dedup::ExactSet::open(
            &Path::new(&self.base_path)
                .join(name)
                .join("exact-versions.redb"),
            &worker.dedup,
        )
        .expect("open exact index dedup store");
        let mut committed_keys = std::collections::HashSet::new();
        let mut key_bytes = 0usize;

        let paths = vec![self.warc_path.clone()];
        let warc_files = download_all_warc_files(&paths, &self.source_config);
        pin!(warc_files);

        let mut num_inserts_since_commit = 0;

        for file in warc_files.by_ref() {
            let mut batch = Vec::with_capacity(self.settings.batch_size);

            for chunk in file
                .records()
                .filter_map(|record| match record {
                    Ok(record) => Some(record),
                    Err(error) => {
                        tracing::error!("WARC contract error: {error}");
                        None
                    }
                })
                .filter(|record| record.indexing_allowed() && !record.is_revalidation())
                .filter(|record| match &record.response.payload_type {
                    Some(payload_type) => matches!(payload_type, PayloadType::Html),
                    None => true,
                })
                .chunks(self.settings.batch_size)
                .into_iter()
            {
                batch.clear();

                let mut keys = Vec::new();
                let mut batch_keys = std::collections::HashSet::new();
                for record in chunk {
                    let key = crate::crawler::dedup::ExactSet::version_key(&record)
                        .expect("capture dedup contract");
                    if exact.contains(&key).expect("read exact dedup key")
                        || committed_keys.contains(&key)
                        || !batch_keys.insert(key.clone())
                    {
                        continue;
                    }
                    let page = IndexableWebpage::from(record);
                    keys.push(key);
                    batch.push(page);
                }

                let prepared = crate::block_on(worker.prepare_webpages_with_positions(&batch));

                for (position, webpage) in &prepared {
                    if webpage.host_centrality > 0.0 {
                        has_host_centrality = true;
                    }

                    if webpage.page_centrality > 0.0 {
                        has_page_centrality = true;
                    }

                    if !webpage.backlinks().is_empty() {
                        has_backlinks = true;
                    }
                    trace!("inserting webpage: {:?}", webpage.html.url());
                    trace!("title = {:?}", webpage.html.title());
                    trace!("text = {:?}", webpage.html.clean_text());

                    if let Err(err) = index.insert(webpage) {
                        warn!("{:?}", err);
                        panic!();
                    }

                    let key = keys[*position].clone();
                    key_bytes += key.len() + 64;
                    committed_keys.insert(key);
                    num_inserts_since_commit += 1;
                }

                if num_inserts_since_commit >= self.settings.autocommit_after_num_inserts
                    || key_bytes >= worker.dedup.max_memory_bytes
                {
                    index.commit().unwrap();
                    exact
                        .commit(&committed_keys.drain().collect::<Vec<_>>())
                        .expect("persist exact index dedup after index commit");
                    key_bytes = 0;
                    num_inserts_since_commit = 0;
                }
            }
        }
        index.commit().unwrap();
        exact
            .commit(&committed_keys.drain().collect::<Vec<_>>())
            .expect("persist exact index dedup after index commit");

        if !has_host_centrality {
            warn!("no host centrality values found in {}", name);
        }

        if !has_page_centrality && worker.page_centrality_store().is_some() {
            warn!("no page centrality values found in {}", name);
        }

        if !has_backlinks && worker.page_webgraph().is_some() {
            warn!("no backlinks found in {}", name);
        }

        index.inverted_index.merge_into_max_segments(1).unwrap();

        info!("{} done", name);

        index
    }
}

#[cfg(test)]
mod capture_contract_tests {
    #[test]
    fn header_noindex_is_archived_and_excluded_before_index_conversion() {
        let url = url::Url::parse("https://example.org/page").unwrap();
        let capture = crate::capture::fixture(
            &url,
            uuid::Uuid::new_v4().to_string(),
            b"<html><title>Saved</title><body>Archive me</body></html>".to_vec(),
            vec![crate::capture::Header {
                name: "x-robots-tag".into(),
                value_hex: crate::capture::hex(b"AstianGOBot: noindex"),
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
        assert!(record.response.body.contains("Archive me"));
        assert!(!record.indexing_allowed());
        assert!(record.links_allowed());
    }

    #[test]
    fn user_agent_meta_noindex_is_transmitted_by_the_capture_contract() {
        let url = url::Url::parse("https://example.org/page").unwrap();
        let body = "<html><head><title>Saved</title><meta name=AstianGOBot content=NOINDEX></head><body>Original</body></html>";
        let mut capture = crate::capture::fixture(
            &url,
            uuid::Uuid::new_v4().to_string(),
            body.as_bytes().to_vec(),
            Vec::new(),
        );
        let html = crate::webpage::Html::parse_without_text(body, url.as_str()).unwrap();
        capture.metadata.robots = html.robots_for_agent(crate::capture::DEFAULT_AGENT);
        let mut bytes = Vec::new();
        crate::warc::capture::write(
            &mut bytes,
            &capture,
            crate::warc::PayloadType::Html,
            3,
            false,
        )
        .unwrap();
        let record = crate::warc::WarcFile::new(bytes)
            .records()
            .next()
            .unwrap()
            .unwrap();
        assert!(!record.indexing_allowed());
        assert_eq!(record.response.body, body);
    }
}

#[cfg(test)]
mod exact_dedup_contract {
    use super::*;
    #[test]
    fn saturated_filter_preserves_distinct_index_documents_and_never_marks_rejected_versions() {
        let dir = tempfile::tempdir().unwrap();
        let warcs = dir.path().join("warc");
        std::fs::create_dir(&warcs).unwrap();
        let mut writer = crate::warc::WarcWriter::new();
        let mut rejected = None;
        for i in 0..200 {
            let record = crate::warc::WarcRecord {
                request: crate::warc::Request {
                    url: format!("https://example.org/page/{i}?signature=keep"),
                    date: Some(chrono::Utc::now()),
                },
                response: crate::warc::Response {
                    body: format!(
                        "<html><head><title>Page {i}</title></head><body><p>{}</p></body></html>",
                        "An original page with several readable words for indexing. ".repeat(40)
                    ),
                    payload_type: Some(PayloadType::Html),
                },
                metadata: crate::warc::Metadata {
                    fetch_time_ms: 1,
                    capture: None,
                },
            };
            writer.write(&record).unwrap();
            if i == 0 {
                let mut bad = record.clone();
                bad.response.body = "<html><body>missing title</body></html>".into();
                rejected = Some(crate::crawler::dedup::ExactSet::version_key(&bad).unwrap());
                writer.write(&bad).unwrap();
            }
        }
        std::fs::write(warcs.join("fixture.warc.gz"), writer.finish().unwrap()).unwrap();
        let config = super::super::worker::Config {
            dedup: crate::crawler::dedup::Config {
                max_memory_bytes: 8,
                ..Default::default()
            },
            host_centrality_store_path: dir.path().join("centrality").to_str().unwrap().into(),
            page_centrality_store_path: None,
            page_webgraph: None,
            safety_classifier_path: None,
            dual_encoder: None,
        };
        let mut worker = crate::block_on(IndexingWorker::new(config));
        let settings = JobSettings {
            host_centrality_threshold: None,
            minimum_clean_words: None,
            batch_size: 16,
            autocommit_after_num_inserts: 32,
        };
        worker.set_job_settings(settings);
        let job = Job {
            source_config: config::WarcSource::Local(config::LocalConfig {
                folder: warcs.to_str().unwrap().into(),
                names: Vec::new(),
            }),
            warc_path: "fixture.warc.gz".into(),
            base_path: dir.path().join("indexes").to_str().unwrap().into(),
            settings,
        };
        let index = job.process(&worker);
        assert_eq!(index.inverted_index.num_documents(), 200);
        let set = crate::crawler::dedup::ExactSet::open(
            &dir.path()
                .join("indexes/fixture.warc.gz/exact-versions.redb"),
            &worker.dedup,
        )
        .unwrap();
        assert!(!set.contains(&rejected.unwrap()).unwrap());
    }
}
