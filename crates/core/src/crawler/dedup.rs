use std::path::Path;

use anyhow::Result;
use bloom::BytesBloomFilter;
use redb::{ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

const KEYS: TableDefinition<&str, ()> = TableDefinition::new("exact_versions.v1");

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub estimated_items: u64,
    pub false_positive_rate: f64,
    pub max_memory_bytes: usize,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            estimated_items: 1_000_000,
            false_positive_rate: 0.01,
            max_memory_bytes: 8 * 1024 * 1024,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.estimated_items > 0
                && (0.0..1.0).contains(&self.false_positive_rate)
                && self.false_positive_rate > 0.0
                && self.max_memory_bytes >= 8
                && self.max_memory_bytes <= 256 * 1024 * 1024,
            "invalid dedup dimensions"
        );
        Ok(())
    }
}

pub struct ExactSet {
    db: redb::Database,
    bloom: BytesBloomFilter<String>,
}
impl ExactSet {
    pub fn open(path: &Path, config: &Config) -> Result<Self> {
        config.validate()?;
        let db = redb::Database::builder()
            .set_cache_size(4 * 1024 * 1024)
            .create(path)?;
        let tx = db.begin_write()?;
        tx.open_table(KEYS)?;
        tx.commit()?;
        let mut bloom = BytesBloomFilter::with_memory_limit(
            config.estimated_items,
            config.false_positive_rate,
            config.max_memory_bytes,
        );
        for item in db.begin_read()?.open_table(KEYS)?.iter()? {
            let (key, _) = item?;
            bloom.insert_raw(key.value().as_bytes());
        }
        Ok(Self { db, bloom })
    }
    pub fn contains(&self, key: &str) -> Result<bool> {
        if !self.bloom.contains_raw(key.as_bytes()) {
            return Ok(false);
        }
        Ok(self.db.begin_read()?.open_table(KEYS)?.get(key)?.is_some())
    }
    pub fn commit(&mut self, keys: &[String]) -> Result<()> {
        let mut tx = self.db.begin_write()?;
        tx.set_durability(redb::Durability::Paranoid);
        {
            let mut table = tx.open_table(KEYS)?;
            for key in keys {
                table.insert(key.as_str(), ())?;
            }
        }
        tx.commit()?;
        for key in keys {
            self.bloom.insert_raw(key.as_bytes());
        }
        Ok(())
    }
    pub fn version_key(record: &crate::warc::WarcRecord) -> Result<String> {
        let url = crate::capture::url_key(&url::Url::parse(&record.request.url)?)?;
        let payload = crate::capture::sha256(record.response.body.as_bytes());
        let metadata = if let Some(capture) = &record.metadata.capture {
            serde_json::to_vec(&(
                capture.schema_version,
                &capture.response.headers,
                &capture.robots,
                &capture.charset,
            ))?
        } else {
            Vec::new()
        };
        Ok(format!(
            "v1:{url}:{payload}:{}",
            crate::capture::sha256(&metadata)
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saturated_bloom_preserves_every_exact_key_across_restart() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("exact.redb");
        let config = Config {
            max_memory_bytes: 8,
            ..Default::default()
        };
        let mut set = ExactSet::open(&path, &config).unwrap();
        set.bloom.fill();
        for i in 0..1000 {
            let key = format!("https://example.org/{i}?signed={i}");
            assert!(!set.contains(&key).unwrap());
            set.commit(&[key.clone()]).unwrap();
            assert!(set.contains(&key).unwrap());
        }
        drop(set);
        let set = ExactSet::open(&path, &config).unwrap();
        assert!((0..1000).all(|i| set
            .contains(&format!("https://example.org/{i}?signed={i}"))
            .unwrap()));
        assert!(!set.contains("https://example.org/new").unwrap());
    }
}
