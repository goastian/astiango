use crate::{crawler::CrawlDatum, Result};
use redb::{ReadableTable, ReadableTableMetadata, TableDefinition};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::Mutex};

const PAGES: TableDefinition<&str, &[u8]> = TableDefinition::new("index_outbox.v1");
const URLS: TableDefinition<&str, &str> = TableDefinition::new("index_outbox_urls.v1");
const READY: TableDefinition<(u64, &str), ()> = TableDefinition::new("index_outbox_eligibility.v1");
const MAX_BYTES: u64 = 256 * 1024 * 1024;
const MAX_ITEMS: u64 = 256;
#[derive(Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub datum: CrawlDatum,
    pub attempts: u32,
    pub not_before: u64,
    #[serde(default)]
    pub lease_until: u64,
    pub error: Option<String>,
}
pub struct Outbox {
    db: redb::Database,
    writes: Mutex<()>,
}
impl Outbox {
    pub fn open(path: &Path) -> Result<Self> {
        let db = redb::Database::builder()
            .set_cache_size(2 * 1024 * 1024)
            .create(path)?;
        let tx = db.begin_write()?;
        tx.open_table(PAGES)?;
        tx.open_table(READY)?;
        tx.open_table(URLS)?;
        tx.commit()?;
        let tx = db.begin_write()?;
        let ids = tx
            .open_table(PAGES)?
            .iter()?
            .map(|r| r.map(|(k, _)| k.value().to_string()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        for id in ids {
            let bytes = tx
                .open_table(PAGES)?
                .get(id.as_str())?
                .map(|v| v.value().to_vec())
                .unwrap();
            let mut entry: Entry = serde_json::from_slice(&bytes)?;
            tx.open_table(URLS)?.insert(
                crate::capture::url_key(&entry.datum.url)?.as_str(),
                entry.id.as_str(),
            )?;
            if let Some(capture) = &entry.datum.capture {
                tx.open_table(URLS)?
                    .insert(capture.metadata.requested_url.as_str(), entry.id.as_str())?;
            }
            if entry.lease_until > 0 {
                tx.open_table(READY)?
                    .remove((entry.not_before.max(entry.lease_until), id.as_str()))?;
                entry.lease_until = 0;
                let bytes = serde_json::to_vec(&entry)?;
                tx.open_table(PAGES)?
                    .insert(id.as_str(), bytes.as_slice())?;
                tx.open_table(READY)?
                    .insert((entry.not_before, id.as_str()), ())?;
            }
        }
        tx.commit()?;
        Ok(Self {
            db,
            writes: Mutex::new(()),
        })
    }
    pub fn enqueue(&self, datum: CrawlDatum) -> Result<String> {
        let id = datum
            .capture
            .as_ref()
            .map(|c| c.metadata.fetch_id.clone())
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let entry = Entry {
            id: id.clone(),
            datum,
            attempts: 0,
            not_before: 0,
            lease_until: (chrono::Utc::now().timestamp_millis().max(0) as u64)
                .saturating_add(120_000),
            error: None,
        };
        let bytes = serde_json::to_vec(&entry)?;
        let _guard = self.writes.lock().unwrap();
        let mut tx = self.db.begin_write()?;
        tx.set_durability(redb::Durability::Paranoid);
        {
            let mut pages = tx.open_table(PAGES)?;
            if pages.get(id.as_str())?.is_some() {
                return Ok(id);
            }
            let size = pages
                .iter()?
                .map(|r| r.map(|(_, v)| v.value().len() as u64))
                .try_fold(0u64, |a, n| Ok::<_, redb::StorageError>(a + n?))?;
            anyhow::ensure!(
                pages.len()? < MAX_ITEMS && size.saturating_add(bytes.len() as u64) <= MAX_BYTES,
                "live index outbox quota full; fetch admission stopped"
            );
            pages.insert(id.as_str(), bytes.as_slice())?;
        }
        tx.open_table(READY)?
            .insert((entry.lease_until, id.as_str()), ())?;
        tx.open_table(URLS)?.insert(
            crate::capture::url_key(&entry.datum.url)?.as_str(),
            id.as_str(),
        )?;
        if let Some(capture) = &entry.datum.capture {
            tx.open_table(URLS)?
                .insert(capture.metadata.requested_url.as_str(), id.as_str())?;
        }
        tx.commit()?;
        Ok(id)
    }
    pub fn ready(&self, now: u64) -> Result<Option<Entry>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(READY)?;
        let key = table
            .range(..=(now, "\u{10ffff}"))?
            .next()
            .transpose()?
            .map(|(k, _)| k.value().1.to_string());
        key.map(|id| {
            let pages = tx.open_table(PAGES)?;
            let value = pages
                .get(id.as_str())?
                .ok_or_else(|| anyhow::anyhow!("outbox eligibility without page"))?;
            Ok(serde_json::from_slice(value.value())?)
        })
        .transpose()
    }
    pub fn acknowledge(&self, id: &str) -> Result<()> {
        let _guard = self.writes.lock().unwrap();
        let mut tx = self.db.begin_write()?;
        tx.set_durability(redb::Durability::Paranoid);
        let value = tx.open_table(PAGES)?.get(id)?.map(|v| v.value().to_vec());
        if let Some(value) = value {
            let entry: Entry = serde_json::from_slice(&value)?;
            tx.open_table(READY)?
                .remove((entry.not_before.max(entry.lease_until), id))?;
            tx.open_table(PAGES)?.remove(id)?;
            let mut urls = tx.open_table(URLS)?;
            for key in std::iter::once(entry.datum.url.to_string())
                .chain(entry.datum.capture.map(|c| c.metadata.requested_url))
            {
                if urls.get(key.as_str())?.is_some_and(|v| v.value() == id) {
                    urls.remove(key.as_str())?;
                }
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn defer(&self, id: &str, now: u64, error: String) -> Result<()> {
        let _guard = self.writes.lock().unwrap();
        let mut tx = self.db.begin_write()?;
        tx.set_durability(redb::Durability::Paranoid);
        let value = tx
            .open_table(PAGES)?
            .get(id)?
            .map(|v| v.value().to_vec())
            .ok_or_else(|| anyhow::anyhow!("missing pending index page"))?;
        let mut entry: Entry = serde_json::from_slice(&value)?;
        tx.open_table(READY)?
            .remove((entry.not_before.max(entry.lease_until), id))?;
        entry.attempts += 1;
        entry.error = Some(error);
        entry.lease_until = 0;
        entry.not_before = now.saturating_add(crate::crawler::retry::Config::default().delay(
            entry.attempts,
            0,
            id,
        ));
        let bytes = serde_json::to_vec(&entry)?;
        tx.open_table(PAGES)?.insert(id, bytes.as_slice())?;
        if entry.attempts < 6 {
            tx.open_table(READY)?.insert((entry.not_before, id), ())?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn contains_url(&self, url: &url::Url) -> Result<bool> {
        let key = crate::capture::url_key(url)?;
        Ok(self
            .db
            .begin_read()?
            .open_table(URLS)?
            .get(key.as_str())?
            .is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page() -> CrawlDatum {
        let url = "https://example.org/a?sig=keep".parse().unwrap();
        let body = "<html>durable</html>";
        CrawlDatum {
            capture: Some(crate::capture::fixture(
                &url,
                uuid::Uuid::new_v4().to_string(),
                body.as_bytes().to_vec(),
                Vec::new(),
            )),
            url,
            body: body.into(),
            payload_type: crate::warc::PayloadType::Html,
            fetch_time_ms: 1,
            date: chrono::Utc::now(),
        }
    }
    #[test]
    fn index_outbox_survives_restart_and_keeps_failures_until_acknowledgement() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("outbox.redb");
        let outbox = Outbox::open(&path).unwrap();
        let datum = page();
        let url = datum.url.clone();
        let id = outbox.enqueue(datum).unwrap();
        assert!(outbox.contains_url(&url).unwrap());
        assert!(outbox.ready(1).unwrap().is_none());
        drop(outbox);
        let outbox = Outbox::open(&path).unwrap();
        assert_eq!(
            outbox.ready(1).unwrap().unwrap().datum.body,
            "<html>durable</html>"
        );
        outbox.defer(&id, 1000, "replica refused".into()).unwrap();
        assert!(outbox.ready(1000).unwrap().is_none());
        drop(outbox);
        let outbox = Outbox::open(&path).unwrap();
        assert!(outbox.ready(1000).unwrap().is_none());
        assert_eq!(outbox.ready(u64::MAX).unwrap().unwrap().attempts, 1);
        outbox.acknowledge(&id).unwrap();
        assert!(!outbox.contains_url(&url).unwrap());
        assert!(outbox.ready(u64::MAX).unwrap().is_none());
    }
}
