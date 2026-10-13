use super::defaults;
use crate::Result;
use std::{
    fs::File,
    io::{self, BufRead},
};

#[derive(Debug, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode, Clone)]
#[serde(tag = "type")]
pub enum WarcSource {
    HTTP(HttpConfig),
    Local(LocalConfig),
    S3(S3Config),
}

impl WarcSource {
    pub fn paths(&self) -> Result<Vec<String>> {
        let mut warc_paths = Vec::new();
        match &self {
            WarcSource::HTTP(config) => {
                let file = File::open(&config.warc_paths_file)?;
                for line in io::BufReader::new(file).lines() {
                    warc_paths.push(line?);
                }
            }
            WarcSource::Local(config) => {
                warc_paths.clone_from(&config.names);
            }
            WarcSource::S3(config) => {
                let bucket = s3::Bucket::new(
                    &config.bucket,
                    s3::Region::Custom {
                        region: "".to_string(),
                        endpoint: config.endpoint.clone(),
                    },
                    s3::creds::Credentials {
                        access_key: Some(config.access_key.clone()),
                        secret_key: Some(config.secret_key.clone()),
                        security_token: None,
                        session_token: None,
                        expiration: None,
                    },
                )?
                .with_path_style();

                let mut folder = config.folder.clone();

                if !folder.ends_with('/') {
                    folder.push('/');
                }

                let pages = bucket.list_blocking(folder, Some("/".to_string()))?;

                let objects = pages
                    .into_iter()
                    .flat_map(|p| p.contents.into_iter())
                    .collect::<Vec<_>>();

                for p in objects.into_iter().filter_map(|o| {
                    if o.key.ends_with("warc.gz") {
                        Some(o.key)
                    } else {
                        None
                    }
                }) {
                    warc_paths.push(p);
                }
            }
        }

        Ok(warc_paths)
    }
}

#[derive(Debug, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode, Clone)]
pub struct LocalConfig {
    pub folder: String,
    pub names: Vec<String>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode, Clone)]
pub struct HttpConfig {
    pub base_url: String,
    pub warc_paths_file: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, bincode::Encode, bincode::Decode, Clone)]
pub struct S3Config {
    pub bucket: String,
    pub folder: String,
    pub access_key: String,
    pub secret_key: String,
    pub endpoint: String,
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct UserAgent {
    pub full: String,
    pub token: String,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CrawlNetworkPolicy {
    #[default]
    PublicOnly,
    AllowPrivate,
}

pub fn default_max_redirect_hops() -> u8 {
    10
}

#[derive(Debug, serde::Serialize, serde::Deserialize, Clone)]
pub struct CrawlerConfig {
    #[serde(default)]
    pub discovery: crate::crawler::discovery::Config,
    #[serde(default)]
    pub retry: crate::crawler::retry::Config,
    #[serde(default)]
    pub recrawl: crate::crawler::recrawl::Config,
    pub num_worker_threads: usize,
    pub user_agent: UserAgent,

    #[serde(default)]
    pub scheduler: crate::crawler::scheduler::Config,

    #[serde(default)]
    pub robots: crate::crawler::robots_txt::Config,

    #[serde(default)]
    pub network_policy: CrawlNetworkPolicy,

    #[serde(default)]
    pub transport: crate::crawler::transport::TransportConfig,

    #[serde(default)]
    pub body: crate::crawler::body::BodyConfig,

    #[serde(default)]
    pub pipeline: crate::crawler::pipeline::PipelineConfig,

    #[serde(default)]
    pub spool: crate::crawler::spool::SpoolConfig,

    #[serde(default = "default_max_redirect_hops")]
    pub max_redirect_hops: u8,

    #[serde(default = "defaults::Crawler::robots_txt_cache_sec")]
    pub robots_txt_cache_sec: u64,

    #[serde(default = "defaults::Crawler::min_politeness_factor")]
    pub min_politeness_factor: u32,

    #[serde(default = "defaults::Crawler::start_politeness_factor")]
    pub start_politeness_factor: u32,

    #[serde(default = "defaults::Crawler::min_crawl_delay_ms")]
    pub min_crawl_delay_ms: u64,

    #[serde(default = "defaults::Crawler::max_crawl_delay_ms")]
    pub max_crawl_delay_ms: u64,

    #[serde(default = "defaults::Crawler::max_politeness_factor")]
    pub max_politeness_factor: u32,

    #[serde(default = "defaults::Crawler::max_url_slowdown_retry")]
    pub max_url_slowdown_retry: u8,

    pub timeout_seconds: u64,
    pub s3: S3Config,
    pub router_hosts: Vec<String>,
}

#[cfg(any(test, feature = "crawler-bench"))]
impl CrawlerConfig {
    pub fn for_tests() -> Self {
        Self {
            num_worker_threads: 1,
            user_agent: UserAgent {
                full: "test".to_string(),
                token: "test".to_string(),
            },
            network_policy: CrawlNetworkPolicy::default(),
            discovery: crate::crawler::discovery::Config::default(),
            retry: crate::crawler::retry::Config::default(),
            recrawl: crate::crawler::recrawl::Config::default(),
            scheduler: crate::crawler::scheduler::Config::default(),
            robots: crate::crawler::robots_txt::Config::default(),
            transport: crate::crawler::transport::TransportConfig::default(),
            body: crate::crawler::body::BodyConfig::default(),
            pipeline: crate::crawler::pipeline::PipelineConfig::default(),
            spool: crate::crawler::spool::SpoolConfig::default(),
            max_redirect_hops: default_max_redirect_hops(),
            robots_txt_cache_sec: defaults::Crawler::robots_txt_cache_sec(),
            min_politeness_factor: defaults::Crawler::min_politeness_factor(),
            start_politeness_factor: defaults::Crawler::start_politeness_factor(),
            min_crawl_delay_ms: defaults::Crawler::min_crawl_delay_ms(),
            max_crawl_delay_ms: defaults::Crawler::max_crawl_delay_ms(),
            max_politeness_factor: defaults::Crawler::max_politeness_factor(),
            max_url_slowdown_retry: defaults::Crawler::max_url_slowdown_retry(),
            timeout_seconds: defaults::Crawler::timeout_seconds(),
            s3: S3Config {
                bucket: "".to_string(),
                folder: "".to_string(),
                access_key: "".to_string(),
                secret_key: "".to_string(),
                endpoint: "".to_string(),
            },
            router_hosts: Vec::new(),
        }
    }
}
