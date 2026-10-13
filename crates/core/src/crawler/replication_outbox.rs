//! Durable original captures awaiting a replicated ACK. Derived dashboards never own them.
use anyhow::{Context, Result};
use redb::{ReadableTable, ReadableTableMetadata, TableDefinition};
use std::{path::Path, sync::Arc};
const CAPTURES: TableDefinition<&str, &[u8]> = TableDefinition::new("replication-captures-v1");
const VERSION: TableDefinition<&str, u32> = TableDefinition::new("metadata");
const MAX_ENTRIES: u64 = 8;
const MAX_PAYLOAD: usize = 4 * 1024 * 1024;
const MAX_METADATA: usize = 512 * 1024;

pub struct Pending(redb::Database);
impl Pending {
    pub fn open(directory: &Path) -> Result<Self> {
        super::spool::create_durable_directory(directory)?;
        let db = redb::Database::builder().set_cache_size(2 * 1024 * 1024).create(directory.join("replication-outbox.redb"))?;
        let tx = db.begin_write()?;
        {
            let mut version = tx.open_table(VERSION)?;
            let current = version.get("version")?.map(|value| value.value()).unwrap_or(1);
            anyhow::ensure!(current == 1, "unsupported replication outbox version {current}");
            version.insert("version", 1)?;
        }
        tx.open_table(CAPTURES)?;
        tx.commit()?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok(Self(db))
    }
    pub fn save(&self, capture: &crate::capture::Capture) -> Result<()> {
        capture.metadata.validate()?;
        let metadata = serde_json::to_vec(&capture.metadata)?;
        anyhow::ensure!(metadata.len() <= MAX_METADATA && capture.payload.len() <= MAX_PAYLOAD, "replication outbox record exceeds budget");
        let mut bytes = Vec::with_capacity(4 + metadata.len() + capture.payload.len());
        bytes.extend_from_slice(&(metadata.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&metadata);
        bytes.extend_from_slice(&capture.payload);
        let tx = self.0.begin_write()?;
        {
            let mut table = tx.open_table(CAPTURES)?;
            if let Some(existing) = table.get(capture.metadata.fetch_id.as_str())? {
                anyhow::ensure!(existing.value() == bytes, "pending capture conflicts with durable original");
                return Ok(());
            }
            anyhow::ensure!(table.len()? < MAX_ENTRIES, "replication outbox quota reached; stop admission");
            table.insert(capture.metadata.fetch_id.as_str(), bytes.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn load(&self, fetch_id: &str) -> Result<Option<crate::capture::Capture>> {
        let tx = self.0.begin_read()?;
        let table = tx.open_table(CAPTURES)?;
        let Some(value) = table.get(fetch_id)? else { return Ok(None) };
        let value = value.value();
        let length = u32::from_be_bytes(value.get(..4).context("truncated pending capture")?.try_into()?) as usize;
        anyhow::ensure!(length <= MAX_METADATA && value.len() <= 4 + MAX_METADATA + MAX_PAYLOAD, "pending capture budget exceeded");
        let metadata: crate::capture::Metadata = serde_json::from_slice(value.get(4..4 + length).context("truncated pending metadata")?)?;
        let payload = Arc::new(value[4 + length..].to_vec());
        metadata.validate()?;
        anyhow::ensure!(metadata.fetch_id == fetch_id && metadata.payload_bytes == payload.len() && metadata.payload_sha256 == crate::capture::sha256(&payload), "pending capture identity/digest mismatch");
        Ok(Some(crate::capture::Capture {metadata, payload}))
    }
    pub fn remove(&self, fetch_id: &str) -> Result<()> {
        let tx = self.0.begin_write()?;
        tx.open_table(CAPTURES)?.remove(fetch_id)?;
        tx.commit()?;
        Ok(())
    }
    pub fn full(&self) -> Result<bool> { Ok(self.0.begin_read()?.open_table(CAPTURES)?.len()? >= MAX_ENTRIES) }
    pub fn ids(&self) -> Result<Vec<String>> {
        self.0.begin_read()?.open_table(CAPTURES)?.iter()?.map(|entry| entry.map(|(id,_)|id.value().to_owned()).map_err(Into::into)).collect()
    }
}
