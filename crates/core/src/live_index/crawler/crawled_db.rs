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
// along with this program.  If not, see <https://www.gnu.org/licenses/

use dashmap::DashMap;
use redb::ReadableTable;
use url::Url;

use crate::{webpage::url_ext::UrlExt, Result};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use crate::live_index::TTL;

#[derive(Debug, Clone)]
struct InsertionTime {
    time: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
struct TruncatedUrl(String);

impl TruncatedUrl {
    fn new(url: &Url) -> Self {
        let mut url = url.clone().to_string();

        // redb has a hard limit of 4gb per value
        // and is better suited for smaller values.
        // we therefore truncate to ~8kb
        if url.len() > 8192 {
            url = url.chars().take(8192).collect();
        }

        Self(url)
    }
}

impl redb::Key for TruncatedUrl {
    fn compare(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
        a.cmp(b)
    }
}

impl redb::Value for TruncatedUrl {
    type SelfType<'a>
        = TruncatedUrl
    where
        Self: 'a;

    type AsBytes<'a>
        = &'a [u8]
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        let s = std::str::from_utf8(data).unwrap();
        Self(s.to_string())
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        value.0.as_bytes()
    }

    fn type_name() -> redb::TypeName {
        redb::TypeName::new("TruncatedUrl")
    }
}

impl redb::Key for InsertionTime {
    fn compare(a: &[u8], b: &[u8]) -> std::cmp::Ordering {
        a.cmp(b)
    }
}

impl redb::Value for InsertionTime {
    type SelfType<'a>
        = InsertionTime
    where
        Self: 'a;

    type AsBytes<'a>
        = Vec<u8>
    where
        Self: 'a;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        let time = chrono::DateTime::parse_from_rfc3339(std::str::from_utf8(data).unwrap())
            .unwrap()
            .with_timezone(&chrono::Utc);

        Self { time }
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'a,
        Self: 'b,
    {
        let s = value.time.to_rfc3339();
        s.as_bytes().to_vec()
    }

    fn type_name() -> redb::TypeName {
        redb::TypeName::new("InsertionTime")
    }
}

struct InnerDb {
    db: redb::Database,
    last_truncate: chrono::DateTime<chrono::Utc>,
    clock: Arc<dyn crate::crawler::runtime::Clock>,
}
const URLS_V2: redb::TableDefinition<&str, i64> = redb::TableDefinition::new("urls.v2");
const TIMES_V2: redb::TableDefinition<(i64, &str), ()> = redb::TableDefinition::new("times.v2");

impl InnerDb {
    fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::open_with_clock(path, Arc::new(crate::crawler::runtime::SystemClock))
    }
    fn open_with_clock<P: AsRef<Path>>(
        path: P,
        clock: Arc<dyn crate::crawler::runtime::Clock>,
    ) -> Result<Self> {
        std::fs::create_dir_all(path.as_ref())?;
        let file = path.as_ref().join("downloaded.db");
        let backup = path.as_ref().join("downloaded.identity-v1.db");
        if file.exists() && !backup.exists() {
            let temporary = path.as_ref().join("downloaded.identity-v1.tmp");
            std::fs::copy(&file, &temporary)?;
            std::fs::File::open(&temporary)?.sync_all()?;
            std::fs::rename(temporary, backup)?;
            std::fs::File::open(path.as_ref())?.sync_all()?;
        }
        let db = redb::Database::builder()
            .set_cache_size(512 * 1024)
            .create(file)?;
        let tx = db.begin_write()?;
        tx.open_table(Self::urls_table_definition())?;
        tx.open_table(Self::times_table_definition())?;
        tx.open_table(URLS_V2)?;
        tx.open_table(TIMES_V2)?;
        tx.commit()?;
        loop {
            let tx = db.begin_write()?;
            let batch = tx
                .open_table(Self::times_table_definition())?
                .iter()?
                .take(128)
                .map(|r| r.map(|(time, url)| (time.value(), url.value())))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if batch.is_empty() {
                break;
            }
            for (time, url) in batch {
                if let Ok(parsed) = Url::parse(&url.0) {
                    let key = crate::capture::url_key(&parsed)?;
                    let millis = time.time.timestamp_millis();
                    let previous = tx
                        .open_table(URLS_V2)?
                        .get(key.as_str())?
                        .map(|v| v.value());
                    if previous.is_none_or(|previous| previous < millis) {
                        if let Some(previous) = previous {
                            tx.open_table(TIMES_V2)?.remove((previous, key.as_str()))?;
                        }
                        tx.open_table(URLS_V2)?.insert(key.as_str(), millis)?;
                        tx.open_table(TIMES_V2)?
                            .insert((millis, key.as_str()), ())?;
                    }
                }
                tx.open_table(Self::times_table_definition())?
                    .remove(time)?;
            }
            tx.commit()?;
        }
        let now = clock.utc_now();
        Ok(Self {
            db,
            last_truncate: now,
            clock,
        })
    }
    fn urls_table_definition() -> redb::TableDefinition<'static, TruncatedUrl, ()> {
        redb::TableDefinition::new("urls")
    }
    fn times_table_definition() -> redb::TableDefinition<'static, InsertionTime, TruncatedUrl> {
        redb::TableDefinition::new("times")
    }
    fn has_crawled(&self, url: &Url) -> Result<bool> {
        let key = crate::capture::url_key(url)?;
        let now = self.clock.utc_now().timestamp_millis();
        let timestamp = self
            .db
            .begin_read()?
            .open_table(URLS_V2)?
            .get(key.as_str())?
            .map(|v| v.value());
        Ok(timestamp.is_some_and(|time| now.saturating_sub(time) < TTL.as_millis() as i64))
    }
    fn truncate(&mut self, ttl: Duration) -> Result<()> {
        let now = self.clock.utc_now();
        let threshold = now
            .timestamp_millis()
            .saturating_sub(ttl.as_millis() as i64);
        loop {
            let tx = self.db.begin_write()?;
            let batch = tx
                .open_table(TIMES_V2)?
                .range(..=(threshold, "\u{10ffff}"))?
                .take(128)
                .map(|r| r.map(|(key, _)| (key.value().0, key.value().1.to_string())))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            if batch.is_empty() {
                break;
            }
            for (time, key) in batch {
                tx.open_table(TIMES_V2)?.remove((time, key.as_str()))?;
                let latest = tx
                    .open_table(URLS_V2)?
                    .get(key.as_str())?
                    .map(|v| v.value());
                if latest == Some(time) {
                    tx.open_table(URLS_V2)?.remove(key.as_str())?;
                }
            }
            tx.commit()?;
        }
        self.last_truncate = now;
        Ok(())
    }
    fn maybe_truncate(&mut self) -> Result<()> {
        if self
            .clock
            .utc_now()
            .signed_duration_since(self.last_truncate)
            .num_seconds()
            >= 60
        {
            self.truncate(TTL)?;
        }
        Ok(())
    }
    fn insert(&mut self, url: &Url) -> Result<()> {
        let key = crate::capture::url_key(url)?;
        let time = self.clock.utc_now().timestamp_millis();
        let mut tx = self.db.begin_write()?;
        tx.set_durability(redb::Durability::Paranoid);
        let previous = tx
            .open_table(URLS_V2)?
            .get(key.as_str())?
            .map(|v| v.value());
        if let Some(previous) = previous {
            tx.open_table(TIMES_V2)?.remove((previous, key.as_str()))?;
        }
        tx.open_table(URLS_V2)?.insert(key.as_str(), time)?;
        tx.open_table(TIMES_V2)?.insert((time, key.as_str()), ())?;
        tx.commit()?;
        self.maybe_truncate()
    }
}

struct CrawledDb {
    inner: std::sync::Mutex<InnerDb>,
}

impl CrawledDb {
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let db = InnerDb::open(path)?;

        Ok(Self {
            inner: std::sync::Mutex::new(db),
        })
    }

    pub fn has_crawled(&self, url: &Url) -> Result<bool> {
        self.inner.lock().unwrap().has_crawled(url)
    }

    pub fn insert(&self, url: &Url) -> Result<()> {
        self.inner.lock().unwrap().insert(url)
    }
}

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct ShardedHost(String);

impl ShardedHost {
    fn from_url(url: &Url) -> Option<Self> {
        let host = url.normalized_host()?;
        let hash = md5::compute(host);
        let hash = format!("{:x}", hash).chars().take(2).collect();

        Some(Self(hash))
    }
}

pub struct ShardedCrawledDb {
    inner: Arc<DashMap<ShardedHost, CrawledDb>>,
    folder: PathBuf,
}

impl ShardedCrawledDb {
    pub fn open<P: AsRef<Path>>(folder: P) -> Result<Self> {
        if !folder.as_ref().exists() {
            std::fs::create_dir_all(folder.as_ref())?;
        }

        let inner = Arc::new(DashMap::new());
        for entry in std::fs::read_dir(&folder)? {
            let entry = entry?;
            let name = entry.file_name();
            if !entry.file_type()?.is_dir()
                || !name.to_str().is_some_and(|name| {
                    name.len() == 2 && name.chars().all(|c| c.is_ascii_hexdigit())
                })
            {
                continue;
            }
            inner.insert(
                ShardedHost(name.to_str().unwrap().to_string()),
                CrawledDb::open(entry.path())?,
            );
        }

        Ok(Self {
            folder: folder.as_ref().to_path_buf(),
            inner,
        })
    }

    pub fn has_crawled(&self, url: &Url) -> Result<bool> {
        let shard =
            ShardedHost::from_url(url).ok_or(anyhow::anyhow!("Failed to get shard from url"))?;
        match self.inner.get(&shard) {
            Some(db) => db.has_crawled(url),
            None => Ok(false),
        }
    }

    pub fn insert(&self, url: &Url) -> Result<()> {
        let shard =
            ShardedHost::from_url(url).ok_or(anyhow::anyhow!("Failed to get shard from url"))?;

        let db = match self.inner.entry(shard.clone()) {
            dashmap::mapref::entry::Entry::Occupied(entry) => entry.into_ref(),
            dashmap::mapref::entry::Entry::Vacant(entry) => {
                entry.insert(CrawledDb::open(self.folder.join(shard.0))?)
            }
        };
        db.insert(url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_downloaded_db() {
        let dir = crate::gen_temp_dir().unwrap();
        let db = CrawledDb::open(&dir).unwrap();

        let url = Url::parse("https://example.com").unwrap();
        assert!(!db.has_crawled(&url).unwrap());

        db.insert(&url).unwrap();
        assert!(db.has_crawled(&url).unwrap());
    }

    struct TestClock {
        start: std::time::Instant,
        epoch: chrono::DateTime<chrono::Utc>,
        elapsed: std::sync::Mutex<Duration>,
    }
    impl crate::crawler::runtime::Clock for TestClock {
        fn now(&self) -> std::time::Instant {
            self.start + *self.elapsed.lock().unwrap()
        }
        fn utc_now(&self) -> chrono::DateTime<chrono::Utc> {
            self.epoch + chrono::Duration::from_std(*self.elapsed.lock().unwrap()).unwrap()
        }
        fn sleep(&self, delay: Duration) -> crate::crawler::runtime::IoFuture<'_, ()> {
            Box::pin(async move {
                *self.elapsed.lock().unwrap() += delay;
            })
        }
    }
    #[test]
    fn test_truncate_ttl_and_refresh_with_controlled_clock() {
        let dir = tempfile::tempdir().unwrap();
        let clock = Arc::new(TestClock {
            start: std::time::Instant::now(),
            epoch: chrono::Utc::now(),
            elapsed: std::sync::Mutex::new(Duration::ZERO),
        });
        let mut db = InnerDb::open_with_clock(dir.path(), clock.clone()).unwrap();
        let a = Url::parse("https://example.org/a").unwrap();
        let b = Url::parse("https://example.org/b").unwrap();
        db.insert(&a).unwrap();
        db.insert(&b).unwrap();
        *clock.elapsed.lock().unwrap() = TTL - Duration::from_secs(1);
        db.insert(&a).unwrap();
        *clock.elapsed.lock().unwrap() = TTL + Duration::from_secs(1);
        assert!(db.has_crawled(&a).unwrap());
        assert!(!db.has_crawled(&b).unwrap());
        db.truncate(TTL).unwrap();
        assert!(db.has_crawled(&a).unwrap());
        assert_eq!(
            db.db
                .begin_read()
                .unwrap()
                .open_table(TIMES_V2)
                .unwrap()
                .iter()
                .unwrap()
                .count(),
            1
        );
    }

    #[test]
    fn test_sharded_downloaded_db() {
        let dir = crate::gen_temp_dir().unwrap();
        let db = ShardedCrawledDb::open(&dir).unwrap();

        let url = Url::parse("https://example.com").unwrap();
        assert!(!db.has_crawled(&url).unwrap());

        db.insert(&url).unwrap();
        assert!(db.has_crawled(&url).unwrap());

        let url = Url::parse("https://example.com/foo").unwrap();
        assert!(!db.has_crawled(&url).unwrap());

        db.insert(&url).unwrap();
        assert!(db.has_crawled(&url).unwrap());

        let url = Url::parse("https://another_example.com/bar").unwrap();
        assert!(!db.has_crawled(&url).unwrap());
    }
}
