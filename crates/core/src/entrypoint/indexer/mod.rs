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

pub mod derived;
pub mod metrics;
pub mod fixture;
pub mod pipeline;
pub mod indexable_webpage;
pub mod job;
pub mod worker;

use rayon::prelude::*;
use std::thread;

use itertools::Itertools;

pub use crate::entrypoint::indexer::indexable_webpage::IndexableWebpage;
pub use crate::entrypoint::indexer::job::{Job, JobSettings};
pub use crate::entrypoint::indexer::worker::IndexingWorker;

use crate::config::{self, WarcSource};
use crate::index::Index;
use crate::Result;

#[derive(Debug, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode)]
pub struct IndexPointer(String);

impl From<String> for IndexPointer {
    fn from(path: String) -> Self {
        IndexPointer(path)
    }
}

pub fn run(config: &config::IndexerConfig) -> Result<()> {
    config.consumer.validate()?;
    anyhow::ensure!(
        config.batch_size > 0 && config.limit_warc_files != Some(0),
        "invalid indexer batch or file limit"
    );
    let workspace =
        crate::warc::publication::Workspace::open(std::path::Path::new(&config.output_path))?;
    let warc_paths = config
        .warc_source
        .paths()?
        .into_iter()
        .unique()
        .collect::<Vec<_>>();

    let job_config: WarcSource = config.warc_source.clone();

    let worker = crate::block_on(IndexingWorker::new(config.clone().into()));

    let thread_pool = rayon::ThreadPoolBuilder::new()
        .num_threads(
            config
                .consumer
                .workers
                .min(crate::crawler::resources::effective_cpus()),
        )
        .stack_size(config.consumer.stack_bytes)
        .thread_name(move |num| format!("indexer-worker-{num}"))
        .build()?;

    let indexes: Result<Vec<IndexPointer>> = thread_pool.install(|| {
        warc_paths
            .into_par_iter()
            .skip(config.skip_warc_files.unwrap_or(0))
            .take(config.limit_warc_files.unwrap_or(usize::MAX))
            .map(|warc_path| {
                let key = crate::warc::publication::Workspace::key(&job_config, &warc_path)?;
                let job = Job {
                    source_config: job_config.clone(),
                    warc_path,
                    base_path: workspace.jobs.join(key).to_string_lossy().into_owned(),
                    settings: JobSettings {
                        host_centrality_threshold: config.host_centrality_threshold,
                        minimum_clean_words: config.minimum_clean_words,
                        batch_size: config.batch_size,
                        autocommit_after_num_inserts: config.autocommit_after_num_inserts,
                    },
                };
                Ok(IndexPointer(
                    job.try_process(&worker)?
                        .path()
                        .as_os_str()
                        .to_str()
                        .unwrap()
                        .to_string(),
                ))
            })
            .collect()
    });

    let indexes = indexes?;
    anyhow::ensure!(
        !indexes.is_empty(),
        "no WARC inputs selected for index publication"
    );
    let copies = workspace.copies(
        &indexes
            .into_iter()
            .map(|p| std::path::PathBuf::from(p.0))
            .collect::<Vec<_>>(),
    )?;
    let index = merge(
        copies
            .into_iter()
            .map(|p| IndexPointer(p.to_string_lossy().into_owned()))
            .collect(),
    )?;
    let completed = index.path();
    drop(index);
    workspace.publish(&completed)?;

    Ok(())
}

pub fn merge(indexes: Vec<IndexPointer>) -> Result<Index> {
    let _span=metrics::Span::new(6);
    anyhow::ensure!(!indexes.is_empty(), "cannot merge an empty index list");
    let num_indexes = indexes.len();
    let mut it = indexes.into_iter();
    let num_cores = crate::crawler::resources::effective_cpus().min(2);

    let mut threads = Vec::new();

    for _ in 0..(num_cores + 1) {
        let indexes = it
            .by_ref()
            .take(((num_indexes as f64) / (num_cores as f64)).ceil() as usize)
            .collect_vec();

        if indexes.is_empty() {
            break;
        }

        threads.push(thread::spawn(move || -> Result<Index> {
            let mut it = indexes.into_iter();
            let mut index = Index::open(it.next().unwrap().0)?;

            for other in it {
                let other_path = other.0;
                let other = Index::open(&other_path)?;

                index = index.try_merge(other)?;

                if std::path::Path::new(&other_path).exists() {
                    std::fs::remove_dir_all(other_path)?;
                }
            }

            index.inverted_index.merge_into_max_segments(1)?;

            Ok(index)
        }));
    }

    let mut indexes = Vec::new();
    for thread in threads {
        indexes.push(
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("index merge worker panicked; inputs retained"))??,
        );
    }

    let mut it = indexes.into_iter();
    let mut index = it.next().unwrap();

    for other in it {
        let other_path = other.path();
        index = index.try_merge(other)?;
        if other_path.exists() {
            std::fs::remove_dir_all(other_path)?;
        }
    }

    index.inverted_index.merge_into_max_segments(1)?;

    Ok(index)
}
