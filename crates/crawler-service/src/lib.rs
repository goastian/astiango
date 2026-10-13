pub type Result<T, E = anyhow::Error> = std::result::Result<T, E>;
#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Failed to parse WARC file: {0}")]
    WarcParse(String),
    #[error("Failed to download WARC after retries")]
    DownloadFailed,
    #[error("Unknown webpage robots meta tag")]
    UnknownRobotsMetaTag,
}
#[path = "../../core/src/block_on.rs"]
pub mod block_on;
#[path = "../../core/src/capture.rs"]
pub mod capture;
#[path = "../../core/src/crawler/mod.rs"]
pub mod crawler;
#[path = "../../core/src/dated_url.rs"]
pub mod dated_url;
#[path = "../../core/src/enum_map.rs"]
pub mod enum_map;
#[path = "../../core/src/feed/mod.rs"]
pub mod feed;
#[path = "../../core/src/sitemap.rs"]
pub mod sitemap;
#[path = "../../core/src/stopwords.rs"]
pub mod stopwords;
#[path = "../../core/src/warc.rs"]
pub mod warc;
pub use block_on::block_on;
pub mod config;
pub mod webpage;
pub mod distributed {
    pub mod retry_strategy {
        include!("../../core/src/distributed/retry_strategy.rs");
    }
}
#[derive(
    serde::Serialize,
    serde::Deserialize,
    bincode::Encode,
    bincode::Decode,
    PartialEq,
    Eq,
    Debug,
    Clone,
    Hash,
)]
pub enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> OneOrMany<T> {
    pub fn one(self) -> Option<T> {
        match self {
            OneOrMany::One(one) => Some(one),
            OneOrMany::Many(many) => many.into_iter().next(),
        }
    }

    pub fn many(self) -> Vec<T> {
        match self {
            OneOrMany::One(one) => vec![one],
            OneOrMany::Many(many) => many,
        }
    }
}
