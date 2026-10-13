use anyhow::Result;
use redb::{ReadableTable, ReadableTableMetadata, TableDefinition};
use std::{collections::VecDeque, path::Path, sync::Arc};
use url::Url;

const URLS: TableDefinition<(u64, &str, &str), ()> = TableDefinition::new("planner_urls.v1");
const SIGNALS: TableDefinition<&str, &[u8]> = TableDefinition::new("planner_signals.v1");
const HOSTS: TableDefinition<u128, ()> = TableDefinition::new("planner_hosts.v1");
const TENANTS: TableDefinition<&str, ()> = TableDefinition::new("planner_tenants.v1");
const BUDGETS: TableDefinition<u128, u64> = TableDefinition::new("planner_budgets.v1");
const META: TableDefinition<&str, u64> = TableDefinition::new("planner_checkpoints.v1");
const SIGNATURE: TableDefinition<&str, &str> = TableDefinition::new("planner_sources.v1");
pub struct Store {
    db: redb::Database,
}
impl Store {
    pub fn open(path: &Path, signature: &str) -> Result<Arc<Self>> {
        let db = redb::Database::builder()
            .set_cache_size(8 * 1024 * 1024)
            .create(path)?;
        let tx = db.begin_write()?;
        tx.open_table(URLS)?;
        tx.open_table(SIGNALS)?;
        tx.open_table(HOSTS)?;
        tx.open_table(TENANTS)?;
        tx.open_table(BUDGETS)?;
        tx.open_table(META)?;
        {
            let mut source = tx.open_table(SIGNATURE)?;
            let previous = source.get("configuration")?.map(|v| v.value().to_string());
            anyhow::ensure!(
                previous
                    .as_ref()
                    .is_none_or(|previous| previous == signature),
                "planner checkpoint belongs to different inputs/configuration"
            );
            source.insert("configuration", signature)?;
        }
        tx.commit()?;
        Ok(Arc::new(Self { db }))
    }
    pub fn import_signals(&self, path: &Path) -> Result<()> {
        use std::io::{BufRead, Read};
        let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
        let mut batch = Vec::with_capacity(128);
        loop {
            let mut bytes = Vec::new();
            let n = reader.by_ref().take(16_385).read_until(b'\n', &mut bytes)?;
            if n == 0 {
                break;
            }
            anyhow::ensure!(
                n <= 16_384 && bytes.last() == Some(&b'\n'),
                "planner signal record exceeds limit or is incomplete"
            );
            let signal: super::plan_signals::Signal = serde_json::from_slice(&bytes)?;
            anyhow::ensure!(
                [signal.novelty, signal.freshness, signal.cost]
                    .iter()
                    .all(|n| n.is_finite() && *n >= 0.0)
                    && signal.source.len() <= 256,
                "invalid planner candidate signal"
            );
            batch.push(signal);
            if batch.len() == 128 {
                self.signal_batch(&batch)?;
                batch.clear();
            }
        }
        self.signal_batch(&batch)
    }
    fn signal_batch(&self, batch: &[super::plan_signals::Signal]) -> Result<()> {
        let tx = self.db.begin_write()?;
        for signal in batch {
            let key = crate::capture::url_key(&signal.url)?;
            tx.open_table(SIGNALS)?
                .insert(key.as_str(), serde_json::to_vec(signal)?.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn signal(&self, url: &Url) -> Result<Option<super::plan_signals::Signal>> {
        let key = crate::capture::url_key(url)?;
        self.db
            .begin_read()?
            .open_table(SIGNALS)?
            .get(key.as_str())?
            .map(|v| serde_json::from_slice(v.value()).map_err(Into::into))
            .transpose()
    }
    pub fn checkpoint(&self, key: &str) -> Result<u64> {
        Ok(self
            .db
            .begin_read()?
            .open_table(META)?
            .get(key)?
            .map_or(0, |v| v.value()))
    }
    pub fn set_checkpoint(&self, key: &str, value: u64) -> Result<()> {
        let mut tx = self.db.begin_write()?;
        tx.set_durability(redb::Durability::Paranoid);
        tx.open_table(META)?.insert(key, value)?;
        tx.commit()?;
        Ok(())
    }
    pub fn initialize_budgets(&self, budgets: impl Iterator<Item = (u128, u64)>) -> Result<()> {
        if self.checkpoint("budgets_initialized")? != 0 {
            return Ok(());
        }
        for batch in itertools::Itertools::chunks(budgets, 128).into_iter() {
            let tx = self.db.begin_write()?;
            {
                let mut table = tx.open_table(BUDGETS)?;
                for (id, budget) in batch {
                    table.insert(id, budget)?;
                }
            }
            tx.commit()?;
        }
        self.set_checkpoint("budgets_initialized", 1)
    }
    pub fn budget_count(&self) -> Result<u64> {
        Ok(self.db.begin_read()?.open_table(BUDGETS)?.len()?)
    }
    pub fn remaining_budget(&self) -> Result<u64> {
        self.db
            .begin_read()?
            .open_table(BUDGETS)?
            .iter()?
            .try_fold(0, |sum, row| Ok(sum + row?.1.value()))
    }
    pub fn budget_ids(&self) -> BudgetIds {
        BudgetIds {
            store: self,
            cursor: None,
            batch: VecDeque::new(),
            done: false,
        }
    }
    pub fn insert(&self, group: u64, url: &Url, host: u128) -> Result<bool> {
        Ok(self.insert_batch(&[(group, url.clone(), host)])? != 0)
    }
    pub fn insert_batch(&self, records: &[(u64, Url, u128)]) -> Result<usize> {
        anyhow::ensure!(records.len() <= 128, "planner write batch exceeds limit");
        let mut tx = self.db.begin_write()?;
        tx.set_durability(redb::Durability::Paranoid);
        let mut count = 0;
        for (group, url, host) in records {
            let key = crate::capture::url_key(url)?;
            let tenant = super::Domain::from(url);
            if tx
                .open_table(URLS)?
                .get((*group, tenant.as_str(), key.as_str()))?
                .is_some()
            {
                continue;
            }
            let budget = tx.open_table(BUDGETS)?.get(*host)?.map_or(0, |v| v.value());
            if budget == 0 {
                continue;
            }
            tx.open_table(URLS)?
                .insert((*group, tenant.as_str(), key.as_str()), ())?;
            tx.open_table(HOSTS)?.insert(*host, ())?;
            tx.open_table(TENANTS)?.insert(tenant.as_str(), ())?;
            if budget == 1 {
                tx.open_table(BUDGETS)?.remove(*host)?;
            } else {
                tx.open_table(BUDGETS)?.insert(*host, budget - 1)?;
            }
            count += 1;
        }
        tx.commit()?;
        Ok(count)
    }
    pub fn url_count(&self) -> Result<u64> {
        Ok(self.db.begin_read()?.open_table(URLS)?.len()?)
    }
    pub fn host_count(&self) -> Result<u64> {
        Ok(self.db.begin_read()?.open_table(HOSTS)?.len()?)
    }
    pub fn tenant_count(&self) -> Result<u64> {
        Ok(self.db.begin_read()?.open_table(TENANTS)?.len()?)
    }
    pub fn group(self: &Arc<Self>, id: u64) -> Group {
        Group {
            store: self.clone(),
            id,
        }
    }
}
pub struct BudgetIds<'a> {
    store: &'a Store,
    cursor: Option<u128>,
    batch: VecDeque<u128>,
    done: bool,
}
impl Iterator for BudgetIds<'_> {
    type Item = u128;
    fn next(&mut self) -> Option<u128> {
        if self.batch.is_empty() && !self.done {
            let tx = self.store.db.begin_read().expect("planner budget read");
            let table = tx.open_table(BUDGETS).expect("planner budget table");
            let from = self.cursor.map_or(0, |n| n.saturating_add(1));
            self.batch = table
                .range(from..)
                .expect("planner budget cursor")
                .take(128)
                .map(|r| r.expect("planner budget row").0.value())
                .collect();
            self.done = self.batch.is_empty();
        }
        let id = self.batch.pop_front()?;
        self.cursor = Some(id);
        Some(id)
    }
}
#[derive(Clone)]
pub struct Group {
    pub id: u64,
    store: Arc<Store>,
}
impl Group {
    pub fn iter(&self) -> GroupIter {
        GroupIter {
            group: self.clone(),
            cursor: None,
            batch: VecDeque::new(),
            done: false,
        }
    }
    pub fn next_job(
        &self,
        cursor: &Option<(String, String)>,
        max_urls: usize,
        max_bytes: usize,
    ) -> Result<Option<(super::Domain, Vec<Url>, (String, String))>> {
        let tx = self.store.db.begin_read()?;
        let table = tx.open_table(URLS)?;
        let (tenant, url) = cursor.clone().unwrap_or_default();
        let mut selected_tenant = None;
        let mut urls = Vec::new();
        let mut bytes = 0;
        let mut last = None;
        for item in table.range(
            (self.id, tenant.as_str(), url.as_str())..=(self.id, "\u{10ffff}", "\u{10ffff}"),
        )? {
            let (key, _) = item?;
            let (_, tenant, url) = key.value();
            if cursor.as_ref().is_some_and(|c| c.0 == tenant && c.1 == url) {
                continue;
            }
            if selected_tenant.as_ref().is_some_and(|t| t != tenant)
                || urls.len() >= max_urls
                || bytes + url.len() > max_bytes
            {
                break;
            }
            selected_tenant.get_or_insert_with(|| tenant.to_string());
            bytes += url.len();
            urls.push(Url::parse(url)?);
            last = Some((tenant.to_string(), url.to_string()));
        }
        Ok(selected_tenant.map(|tenant| (super::Domain::from(tenant), urls, last.unwrap())))
    }
}
pub struct GroupIter {
    group: Group,
    cursor: Option<(String, String)>,
    batch: VecDeque<(String, String)>,
    done: bool,
}
impl Iterator for GroupIter {
    type Item = Url;
    fn next(&mut self) -> Option<Self::Item> {
        if self.batch.is_empty() && !self.done {
            let tx = self.group.store.db.begin_read().expect("planner URL read");
            let table = tx.open_table(URLS).expect("planner URL table");
            let (tenant, url) = self.cursor.clone().unwrap_or_default();
            self.batch = table
                .range(
                    (self.group.id, tenant.as_str(), url.as_str())
                        ..=(self.group.id, "\u{10ffff}", "\u{10ffff}"),
                )
                .expect("planner URL cursor")
                .filter_map(|r| {
                    let (key, _) = r.expect("planner URL row");
                    let (_, tenant, url) = key.value();
                    if self
                        .cursor
                        .as_ref()
                        .is_some_and(|c| c.0 == tenant && c.1 == url)
                    {
                        None
                    } else {
                        Some((tenant.to_string(), url.to_string()))
                    }
                })
                .take(128)
                .collect();
            self.done = self.batch.is_empty();
        }
        let (tenant, url) = self.batch.pop_front()?;
        self.cursor = Some((tenant, url.clone()));
        Some(Url::parse(&url).expect("durable planner URL identity"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn checkpoints_replay_idempotently_and_jobs_obey_byte_and_tenant_limits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plan.redb");
        let store = Store::open(&path, "same-input").unwrap();
        store
            .initialize_budgets([(1, 1000), (2, 1000)].into_iter())
            .unwrap();
        let records: Vec<_> = (0..128)
            .map(|i| {
                (
                    u64::from(i % 2 == 0),
                    format!(
                        "https://{}.github.io/page/{i:05}?signed={i}",
                        if i % 2 == 0 { "alice" } else { "bob" }
                    )
                    .parse()
                    .unwrap(),
                    if i % 2 == 0 { 1 } else { 2 },
                )
            })
            .collect();
        assert_eq!(store.insert_batch(&records).unwrap(), 128);
        assert_eq!(store.insert_batch(&records).unwrap(), 0);
        assert_eq!(store.host_count().unwrap(), 2);
        assert_eq!(store.tenant_count().unwrap(), 2);
        store.set_checkpoint("processed", 128).unwrap();
        drop(store);
        let store = Store::open(&path, "same-input").unwrap();
        assert_eq!(store.checkpoint("processed").unwrap(), 128);
        let group = store.group(1);
        let mut cursor = None;
        let mut total = 0;
        while let Some((tenant, urls, next)) = group.next_job(&cursor, 7, 512).unwrap() {
            assert_eq!(tenant.as_str(), "alice.github.io");
            assert!(urls.len() <= 7 && urls.iter().map(|u| u.as_str().len()).sum::<usize>() <= 512);
            total += urls.len();
            cursor = Some(next);
        }
        assert_eq!(total, 64);
        drop(group);
        drop(store);
        assert!(Store::open(&path, "different-input").is_err());
    }
}
