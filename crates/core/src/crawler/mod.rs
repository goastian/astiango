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

//! # Crawler
//!
//! The crawler is responsible for fetching webpages and storing them in WARC files
//! for later processing.
//!
//! Before starting a crawl, a plan needs to be created. This plan is then used by
//! the crawler coordinator to assign sites to crawl to different workers.
//! A site is only assigned to one worker at a time for politeness.

use std::{collections::VecDeque, future::Future, net::SocketAddr, sync::Arc};

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;

use anyhow::anyhow;
use robot_client::RobotClient;
use url::Url;

use crate::{config::CrawlerConfig, warc, webpage::url_ext::UrlExt};

use self::{warc_writer::WarcWriter, worker::WorkerThread};
pub use outcome::{CrawlCounters, FetchOutcome, JobOutcome, JobStatus, UrlOutcome};
pub use worker::JobExecutor;

pub mod body;
pub mod coordinator;
pub mod extraction;
pub mod robots_txt;
pub mod router;
pub use router::Router;
#[cfg(feature = "crawler-bench")]
pub mod benchmark;
#[cfg(feature = "crawler-bench")]
pub mod body_benchmark;
#[cfg(feature = "crawler-bench")]
pub mod extraction_benchmark;
mod file_queue;
pub mod local;
mod observer;
pub mod outcome;
pub mod planner;
pub mod robot_client;
pub mod runtime;
pub mod transport;
#[cfg(feature = "crawler-bench")]
pub mod transport_benchmark;
mod wander_prioritiser;
mod warc_writer;
mod worker;

pub use coordinator::CrawlCoordinator;

pub const MAX_URL_LEN_BYTES: usize = 8192;

pub const MAX_OUTGOING_URLS_PER_PAGE: usize = 512;
pub const MAX_CONTENT_LENGTH: usize = 32 * 1024 * 1024; // 32 MB

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Body(#[from] body::BodyError),

    #[error("invalid content type: {0}")]
    InvalidContentType(String),

    #[error("fetch failed: {status_code}")]
    FetchFailed {
        status_code: reqwest::StatusCode,
        headers: reqwest::header::HeaderMap,
    },

    #[error("content too large")]
    ContentTooLarge,

    #[error("couldn't read response body")]
    ResponseBodyReadFailed,

    #[error("invalid politeness factor")]
    InvalidPolitenessFactor,

    #[error("invalid redirect")]
    InvalidRedirect,

    #[error("couldn't parse html")]
    InvalidHtml,

    #[error("request path is disallowed by robots.txt")]
    DisallowedPath,

    #[error("robots.txt is temporarily unavailable")]
    RobotsUnavailable,

    #[error("destination rejected by network policy: {0}")]
    NonPublicDestination(String),

    #[error("an error occurred: {0}")]
    Anyhow(#[from] anyhow::Error),
}

type Result<T, E = Error> = std::result::Result<T, E>;

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    bincode::Encode,
    bincode::Decode,
)]
struct Site(String);

#[derive(
    Clone,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    bincode::Encode,
    bincode::Decode,
)]
pub struct Domain(String);

impl From<&Url> for Domain {
    fn from(url: &Url) -> Self {
        Self(url.icann_domain().unwrap_or_default().to_string())
    }
}

impl From<Url> for Domain {
    fn from(url: Url) -> Self {
        Self::from(&url)
    }
}

impl From<String> for Domain {
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl Domain {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode, Debug, Clone)]
pub struct WeightedUrl {
    #[bincode(with_serde)]
    pub url: Url,
    pub weight: f64,
}

impl PartialEq for WeightedUrl {
    fn eq(&self, other: &Self) -> bool {
        self.url == other.url
    }
}

impl Eq for WeightedUrl {}

impl std::hash::Hash for WeightedUrl {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.url.hash(state);
    }
}

/// All urls in a job must be from the same domain and only one job per domain.
/// at a time. This ensures that we stay polite when crawling.
#[derive(serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode, Debug, Clone)]
pub struct Job {
    pub domain: Domain,
    pub urls: VecDeque<WeightedUrl>,
    pub wandering_urls: u64,
}

#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    serde::Serialize,
    serde::Deserialize,
    bincode::Encode,
    bincode::Decode,
)]
pub struct UrlString(String);

impl From<&Url> for UrlString {
    fn from(url: &Url) -> Self {
        Self(url.as_str().to_string())
    }
}

impl From<Url> for UrlString {
    fn from(url: Url) -> Self {
        Self(url.as_str().to_string())
    }
}

impl TryFrom<&UrlString> for Url {
    type Error = anyhow::Error;
    fn try_from(url: &UrlString) -> Result<Self, Self::Error> {
        Ok(Url::parse(&url.0)?)
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode)]
pub struct UrlToInsert {
    pub url: UrlString,
    pub weight: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode)]
pub struct DiscoveredUrls {
    pub urls: HashMap<Domain, Vec<UrlToInsert>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode)]
pub struct DomainCrawled {
    pub domain: Domain,
    pub budget_used: f64,
}

pub struct RetrieableUrl {
    weighted_url: WeightedUrl,
    retries: u8,
}

impl RetrieableUrl {
    pub fn url(&self) -> &Url {
        &self.weighted_url.url
    }
}

impl From<WeightedUrl> for RetrieableUrl {
    fn from(weighted_url: WeightedUrl) -> Self {
        Self {
            weighted_url,
            retries: 0,
        }
    }
}

pub struct WorkerJob {
    pub domain: Domain,
    pub urls: VecDeque<RetrieableUrl>,
    pub wandering_urls: u64,
}

impl From<Job> for WorkerJob {
    fn from(value: Job) -> Self {
        Self {
            domain: value.domain,
            urls: value.urls.into_iter().map(RetrieableUrl::from).collect(),
            wandering_urls: value.wandering_urls,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CrawlDatum {
    pub url: Url,
    pub payload_type: warc::PayloadType,
    pub body: String,
    pub fetch_time_ms: u64,
    pub date: chrono::DateTime<chrono::Utc>,
}

pub struct Crawler {
    writer: Arc<WarcWriter>,
    handles: tokio::task::JoinSet<Result<()>>,
}

impl Crawler {
    pub async fn new(config: CrawlerConfig) -> Result<Self> {
        let writer = Arc::new(WarcWriter::new(config.s3.clone()));
        let mut handles = tokio::task::JoinSet::new();
        let mut router_hosts = Vec::new();
        let client = RobotClient::new(&config)?;

        for host in &config.router_hosts {
            router_hosts.push(
                host.parse::<SocketAddr>()
                    .map_err(|e| Error::from(anyhow!(e)))?,
            );
        }

        for _ in 0..config.num_worker_threads {
            let worker = WorkerThread::new(
                Arc::clone(&writer),
                client.clone(),
                config.clone(),
                router_hosts.clone(),
            )?;

            handles.spawn(worker.run());
        }

        Ok(Self { writer, handles })
    }

    pub async fn run(mut self) -> Result<()> {
        let mut failure = None;
        while let Some(result) = self.handles.join_next().await {
            let error = match result {
                Ok(Ok(())) => None,
                Ok(Err(error)) => Some(error),
                Err(error) if error.is_cancelled() && failure.is_some() => None,
                Err(error) => Some(Error::from(anyhow!("Crawler worker failed: {error}"))),
            };
            if let Some(error) = error {
                failure.get_or_insert(error);
                self.handles.abort_all();
            }
        }
        self.writer.finish().await?;
        failure.map_or(Ok(()), Err)
    }
}

pub trait DatumSink: Send + Sync {
    fn write(&self, crawl_datum: CrawlDatum) -> impl Future<Output = Result<()>> + Send;
    /// Reuse the ephemeral extraction when the sink also discovers links.
    /// Existing sinks retain their original contract and durable acknowledgements.
    fn write_extracted(
        &self,
        datum: CrawlDatum,
        _extraction: Option<&extraction::CrawlExtraction>,
    ) -> impl Future<Output = Result<()>> + Send {
        self.write(datum)
    }
    /// Confirm all preceding writes; the default requires write itself to be durable.
    fn checkpoint(&self) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }
    fn finish(&self) -> impl Future<Output = Result<()>> + Send;
}

/// Legacy text decoder with default wire, decoded and time limits per call.
/// Status and MIME admission remain the caller's responsibility. The returned
/// String has no shared reservation; production uses a shared BodyReader and
/// retains its BodyLease through parsing and sink submission instead.
#[deprecated(note = "use a shared BodyReader with explicit BodyKind and retain its BodyLease")]
pub async fn encoded_body(res: reqwest::Response) -> Result<String> {
    let reader = body::BodyReader::new(body::BodyConfig::default())?;
    let admitted = reader.read_legacy(res).await?;
    Ok(admitted.into_parts().0)
}
