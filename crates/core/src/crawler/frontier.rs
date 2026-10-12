use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use redb::{Database, ReadableTable, TableDefinition, WriteTransaction};
use serde::{Deserialize, Serialize};
use url::Url;

use super::{
    observer::Observer, spool::Receipt, Domain, FetchOutcome, Job, JobOutcome, JobStatus,
    WeightedUrl,
};

const URLS: TableDefinition<&str, &[u8]> = TableDefinition::new("urls.v1");
const ORIGINS: TableDefinition<&str, &[u8]> = TableDefinition::new("sites.v2");
const ELIGIBLE: TableDefinition<(&str, &str), u64> = TableDefinition::new("url_eligibility.v2");
const READY: TableDefinition<(u64, u64), &str> = TableDefinition::new("site_eligibility.v2");
const LEASES: TableDefinition<&str, &str> = TableDefinition::new("leases.v1");
const ACTIVE: TableDefinition<&str, ()> = TableDefinition::new("active_sites.v2");
const COMMITS: TableDefinition<&str, &[u8]> = TableDefinition::new("commits.v1");
const OUTBOX: TableDefinition<&str, &[u8]> = TableDefinition::new("outbox.v1");
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata.v1");
const IDENTITY_MAP: TableDefinition<&str, &[u8]> =
    TableDefinition::new("identity_correspondence.v2");
const LEGACY_ORIGINS: TableDefinition<&str, &[u8]> = TableDefinition::new("origins.v1");
const HTTP_ORIGINS: TableDefinition<&str, &[u8]> = TableDefinition::new("http_origins.v1");
const HTTP_READY: TableDefinition<(&str, u64, u64), &str> =
    TableDefinition::new("http_origin_eligibility.v1");
const HTTP_URLS: TableDefinition<(&str, &str), u64> = TableDefinition::new("http_origin_urls.v1");
const HTTP_ACTIVE: TableDefinition<&str, &str> = TableDefinition::new("active_http_origins.v1");
fn legacy_site_version() -> u32 {
    1
}
pub const CACHE_BYTES: usize = 16 * 1024 * 1024;
pub const BATCH_SIZE: usize = 128;

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Stats {
    pub schema_version: u32,
    pub identity_version: u32,
    #[serde(default = "legacy_site_version")]
    pub site_identity_version: u32,
    #[serde(default)]
    pub identity_mapped_urls: u64,
    #[serde(default)]
    pub identity_migration_complete: bool,
    pub cache_bytes: usize,
    pub batch_size: usize,
    pub round: u64,
    pub sequence: u64,
    pub urls: u64,
    pub pending_urls: u64,
    pub leased_urls: u64,
    pub committed_urls: u64,
    pub terminal_urls: u64,
    pub known_sites: u64,
    pub pending_sites: u64,
    pub active_sites: u64,
    pub outbox_entries: u64,
    pub commits: u64,
    pub transactions: u64,
    pub recovered_leases: u64,
    pub reconciled_captures: u64,
    pub migrated_jobs: u64,
    pub migration_complete: bool,
}

#[cfg(test)]
#[path = "frontier_tests.rs"]
mod tests;

#[derive(Clone, Default)]
pub struct Metrics(Arc<Mutex<Stats>>);
impl Metrics {
    pub fn snapshot(&self) -> Stats {
        self.0.lock().unwrap().clone()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
enum State {
    Pending,
    Leased,
    Committed,
    Terminal,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UrlRow {
    url: String,
    origin: String,
    site: String,
    round: u64,
    state: State,
    fetch_id: Option<String>,
    reason: Option<String>,
    not_before: u64,
    #[serde(default)]
    sitemap: bool,
    #[serde(default)]
    continuation: Option<super::RedirectContinuation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct OriginRow {
    sequence: u64,
    round: u64,
    state: String,
    seed: String,
    admitted: u64,
    done: u64,
    not_before: u64,
}

#[derive(Serialize, Deserialize)]
struct HttpOriginRow {
    site: String,
    round: u64,
    sequence: u64,
    not_before: u64,
}

#[derive(Serialize, Deserialize)]
struct CommitRow {
    url: String,
    receipt: Receipt,
}

#[derive(Serialize, Deserialize)]
struct Discovery {
    internal: Vec<Url>,
    external: Vec<Url>,
}

pub struct Frontier {
    pub path: PathBuf,
    db: Database,
    capacity: usize,
    budget: u64,
    observer: Option<Arc<Observer>>,
    pub metrics: Metrics,
    writes: Mutex<()>,
}

fn read<T: serde::de::DeserializeOwned>(
    tx: &WriteTransaction,
    table: TableDefinition<&str, &[u8]>,
    key: &str,
) -> Result<Option<T>> {
    let table = tx.open_table(table)?;
    let value = table.get(key)?;
    value
        .map(|value| serde_json::from_slice(value.value()).map_err(Into::into))
        .transpose()
}

fn put<T: Serialize>(
    tx: &WriteTransaction,
    table: TableDefinition<&str, &[u8]>,
    key: &str,
    value: &T,
) -> Result<()> {
    tx.open_table(table)?
        .insert(key, serde_json::to_vec(value)?.as_slice())?;
    Ok(())
}

fn millis(time: chrono::DateTime<chrono::Utc>) -> u64 {
    time.timestamp_millis().max(0) as u64
}

impl Frontier {
    pub(super) fn open(
        path: PathBuf,
        capacity: usize,
        budget: u64,
        observer: Arc<Observer>,
    ) -> Result<Self> {
        Self::open_inner(path, capacity, budget, Some(observer))
    }

    pub fn open_database(path: PathBuf, capacity: usize, budget: u64) -> Result<Self> {
        Self::open_inner(path, capacity, budget, None)
    }

    fn open_inner(
        legacy_path: PathBuf,
        capacity: usize,
        budget: u64,
        observer: Option<Arc<Observer>>,
    ) -> Result<Self> {
        anyhow::ensure!(capacity > 0 && budget > 0, "invalid frontier limits");
        let path = legacy_path.with_extension("redb");
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        if path.exists() && !path.with_extension("identity-v1.redb").exists() {
            let backup = path.with_extension("identity-v1.redb");
            let temporary = path.with_extension("identity-v1.redb.tmp");
            fs::copy(&path, &temporary)?;
            fs::File::open(&temporary)?.sync_all()?;
            fs::rename(temporary, backup)?;
            if let Some(parent) = path.parent() {
                fs::File::open(parent)?.sync_all()?;
            }
        }
        let db = Database::builder()
            .set_cache_size(CACHE_BYTES)
            .create(&path)?;
        let tx = db.begin_write()?;
        tx.open_table(URLS)?;
        tx.open_table(ORIGINS)?;
        tx.open_table(ELIGIBLE)?;
        tx.open_table(READY)?;
        tx.open_table(LEASES)?;
        tx.open_table(COMMITS)?;
        tx.open_table(OUTBOX)?;
        tx.open_table(META)?;
        tx.open_table(ACTIVE)?;
        tx.open_table(IDENTITY_MAP)?;
        tx.open_table(HTTP_ORIGINS)?;
        tx.open_table(HTTP_READY)?;
        tx.open_table(HTTP_URLS)?;
        tx.open_table(HTTP_ACTIVE)?;
        let stats: Stats = read(&tx, META, "stats")?.unwrap_or_else(|| Stats {
            schema_version: 1,
            identity_version: crate::capture::IDENTITY_VERSION,
            site_identity_version: super::identity::SITE_VERSION,
            identity_migration_complete: true,
            cache_bytes: CACHE_BYTES,
            batch_size: BATCH_SIZE,
            round: 1,
            ..Default::default()
        });
        anyhow::ensure!(
            stats.schema_version == 1 && stats.identity_version == crate::capture::IDENTITY_VERSION,
            "unsupported frontier or URL identity version: {}/{}",
            stats.schema_version,
            stats.identity_version
        );
        anyhow::ensure!(
            (1..=super::identity::SITE_VERSION).contains(&stats.site_identity_version),
            "unsupported site identity version: {}",
            stats.site_identity_version
        );
        put(&tx, META, "stats", &stats)?;
        tx.commit()?;
        if let Some(parent) = path.parent() {
            fs::File::open(parent)?.sync_all()?;
        }
        let frontier = Self {
            path,
            db,
            capacity,
            budget,
            observer,
            metrics: Metrics(Arc::new(Mutex::new(stats))),
            writes: Mutex::new(()),
        };
        frontier.migrate_site_identity()?;
        frontier.migrate_http_origins()?;
        if !frontier.metrics.snapshot().migration_complete {
            frontier.migrate(&legacy_path)?;
        }
        frontier.recover_leases()?;
        frontier.recover_jobs()?;
        frontier.drain_outbox()?;
        frontier.publish();
        Ok(frontier)
    }

    fn migrate_site_identity(&self) -> Result<()> {
        if self.metrics.snapshot().site_identity_version == super::identity::SITE_VERSION {
            return Ok(());
        }
        self.transaction(|tx, stats| {
            if read::<String>(tx, META, "site-migration-cursor")?.is_none() {
                stats.known_sites = 0;
                stats.pending_sites = 0;
                stats.active_sites = 0;
                put(tx, META, "site-migration-cursor", &String::new())?;
            }
            Ok(())
        })?;
        loop {
            let complete = self.transaction(|tx, stats| {
                let cursor: String = read(tx, META, "site-migration-cursor")?.unwrap_or_default();
                let keys = {
                    let table = tx.open_table(URLS)?;
                    table.range::<&str>((std::ops::Bound::Excluded(cursor.as_str()), std::ops::Bound::Unbounded))?
                        .take(BATCH_SIZE).map(|entry| Ok(entry?.0.value().to_owned())).collect::<Result<Vec<_>>>()?
                };
                if keys.is_empty() {
                    stats.site_identity_version = super::identity::SITE_VERSION;
                    stats.identity_migration_complete = true;
                    return Ok(true);
                }
                for key in &keys {
                    let mut row: UrlRow = read(tx, URLS, key)?.context("identity migration URL vanished")?;
                    let identity = super::identity::UrlIdentity::from_url(&Url::parse(key)?)?;
                    anyhow::ensure!(identity.url == *key, "identity migration would change a URL key");
                    if row.site != identity.tenant {
                        put(tx, IDENTITY_MAP, key, &serde_json::json!({"url": key, "old_site": row.site, "new_site": identity.tenant,
                            "fetch_id": row.fetch_id, "url_version": 1, "old_site_version": 1, "new_site_version": 2}))?;
                        stats.identity_mapped_urls += 1;
                    }
                    if let Some(old_site) = read::<OriginRow>(tx, LEGACY_ORIGINS, &row.site)? {
                        if old_site.round == row.round { row.not_before = row.not_before.max(old_site.not_before); }
                    }
                    row.site = identity.tenant;
                    let mut site: OriginRow = match read(tx, ORIGINS, &row.site)? {
                        Some(site) => site,
                        None => {
                            stats.sequence += 1; stats.known_sites += 1;
                            OriginRow { sequence: stats.sequence, round: stats.round, state: "done".into(), seed: key.clone(), admitted: 0, done: 0, not_before: 0 }
                        }
                    };
                    if row.round == stats.round {
                        site.admitted += 1;
                        if matches!(row.state, State::Pending | State::Leased) {
                            if site.state == "done" { site.state = "pending".into(); site.not_before = row.not_before; stats.pending_sites += 1; }
                            else { tx.open_table(READY)?.remove((site.not_before, site.sequence))?; site.not_before = site.not_before.min(row.not_before); }
                            tx.open_table(READY)?.insert((site.not_before, site.sequence), row.site.as_str())?;
                            if row.state == State::Pending { tx.open_table(ELIGIBLE)?.insert((row.site.as_str(), key.as_str()), row.not_before)?; }
                        } else { site.done += 1; }
                    }
                    put(tx, ORIGINS, &row.site, &site)?; put(tx, URLS, key, &row)?;
                }
                put(tx, META, "site-migration-cursor", keys.last().unwrap())?;
                Ok(false)
            })?;
            if complete {
                break;
            }
        }
        Ok(())
    }

    fn queue_http_origin(tx: &WriteTransaction, stats: &mut Stats, row: &UrlRow) -> Result<()> {
        if row.state != State::Pending {
            return Ok(());
        }
        tx.open_table(HTTP_URLS)?
            .insert((row.origin.as_str(), row.url.as_str()), row.not_before)?;
        let mut origin = match read::<HttpOriginRow>(tx, HTTP_ORIGINS, &row.origin)? {
            Some(origin) if origin.round == row.round => origin,
            _ => {
                stats.sequence += 1;
                HttpOriginRow {
                    site: row.site.clone(),
                    round: row.round,
                    sequence: stats.sequence,
                    not_before: row.not_before,
                }
            }
        };
        tx.open_table(HTTP_READY)?.remove((
            origin.site.as_str(),
            origin.not_before,
            origin.sequence,
        ))?;
        origin.not_before = origin.not_before.max(row.not_before);
        put(tx, HTTP_ORIGINS, &row.origin, &origin)?;
        let active = tx
            .open_table(HTTP_ACTIVE)?
            .get(row.site.as_str())?
            .map(|v| v.value().to_owned());
        if active.as_deref() != Some(&row.origin) {
            tx.open_table(HTTP_READY)?.insert(
                (row.site.as_str(), origin.not_before, origin.sequence),
                row.origin.as_str(),
            )?;
        }
        Ok(())
    }

    fn reschedule_http_origin(
        tx: &WriteTransaction,
        stats: &mut Stats,
        key: &str,
        due: u64,
    ) -> Result<()> {
        let Some(mut origin): Option<HttpOriginRow> = read(tx, HTTP_ORIGINS, key)? else {
            return Ok(());
        };
        tx.open_table(HTTP_READY)?.remove((
            origin.site.as_str(),
            origin.not_before,
            origin.sequence,
        ))?;
        origin.not_before = due;
        stats.sequence += 1;
        origin.sequence = stats.sequence;
        let remaining = tx
            .open_table(HTTP_URLS)?
            .range((key, "")..=(key, "\u{10ffff}"))?
            .next()
            .transpose()?
            .is_some();
        if remaining {
            tx.open_table(HTTP_READY)?
                .insert((origin.site.as_str(), due, origin.sequence), key)?;
        }
        put(tx, HTTP_ORIGINS, key, &origin)
    }

    fn next_http_origin(tx: &WriteTransaction, site: &str) -> Result<Option<(u64, String)>> {
        Ok(tx
            .open_table(HTTP_READY)?
            .range((site, 0, 0)..=(site, u64::MAX, u64::MAX))?
            .next()
            .transpose()?
            .map(|(key, origin)| (key.value().1, origin.value().to_owned())))
    }

    fn migrate_http_origins(&self) -> Result<()> {
        let complete = {
            let tx = self.db.begin_read()?;
            let meta = tx.open_table(META)?;
            let value = meta.get("http-origin-index-complete")?;
            value
                .map(|v| serde_json::from_slice::<bool>(v.value()))
                .transpose()?
                .unwrap_or(false)
        };
        if complete {
            return Ok(());
        }
        loop {
            let complete = self.transaction(|tx, stats| {
                let cursor: String =
                    read(tx, META, "http-origin-index-cursor")?.unwrap_or_default();
                let keys = tx
                    .open_table(URLS)?
                    .range::<&str>((
                        std::ops::Bound::Excluded(cursor.as_str()),
                        std::ops::Bound::Unbounded,
                    ))?
                    .take(BATCH_SIZE)
                    .map(|entry| Ok(entry?.0.value().to_owned()))
                    .collect::<Result<Vec<_>>>()?;
                for key in &keys {
                    let row: UrlRow =
                        read(tx, URLS, key)?.context("missing URL during HTTP origin migration")?;
                    if row.round == stats.round {
                        Self::queue_http_origin(tx, stats, &row)?;
                    }
                }
                if keys.is_empty() {
                    put(tx, META, "http-origin-index-complete", &true)?;
                    return Ok(true);
                }
                put(tx, META, "http-origin-index-cursor", keys.last().unwrap())?;
                Ok(false)
            })?;
            if complete {
                return Ok(());
            }
        }
    }

    fn transaction<T>(
        &self,
        operation: impl FnOnce(&WriteTransaction, &mut Stats) -> Result<T>,
    ) -> Result<T> {
        let _guard = self.writes.lock().unwrap();
        let mut tx = self.db.begin_write()?;
        tx.set_durability(redb::Durability::Paranoid);
        let mut stats: Stats = read(&tx, META, "stats")?.context("missing frontier schema")?;
        let result = operation(&tx, &mut stats)?;
        stats.transactions += 1;
        put(&tx, META, "stats", &stats)?;
        tx.commit()?;
        *self.metrics.0.lock().unwrap() = stats;
        self.publish();
        Ok(result)
    }

    fn publish(&self) {
        if let Some(observer) = &self.observer {
            let stats = self.metrics.snapshot();
            observer.frontier(
                stats.pending_sites as usize,
                stats.known_sites as usize,
                self.capacity,
            );
        }
    }

    fn admissible(url: &Url) -> bool {
        matches!(url.scheme(), "https" | "http")
            && url.username().is_empty()
            && url.password().is_none()
            && url.as_str().len() <= super::MAX_URL_LEN_BYTES
            && !Domain::from(url).as_str().is_empty()
    }

    fn admit(
        &self,
        tx: &WriteTransaction,
        stats: &mut Stats,
        url: &Url,
        internal: bool,
    ) -> Result<bool> {
        if !Self::admissible(url) {
            return Ok(false);
        }
        let key = crate::capture::url_key(url)?;
        let site = Domain::from(url).as_str().to_owned();
        let old: Option<UrlRow> = read(tx, URLS, &key)?;
        if old.as_ref().is_some_and(|row| row.round == stats.round) {
            return Ok(false);
        }
        let mut origin: OriginRow = match read::<OriginRow>(tx, ORIGINS, &site)? {
            Some(row) if row.round == stats.round => {
                if !internal || row.admitted >= self.budget || row.state == "done" {
                    return Ok(false);
                }
                row
            }
            _ => {
                if stats.pending_sites + stats.active_sites >= self.capacity as u64 {
                    return Ok(false);
                }
                stats.sequence += 1;
                tx.open_table(READY)?
                    .insert((0, stats.sequence), site.as_str())?;
                stats.known_sites += 1;
                stats.pending_sites += 1;
                OriginRow {
                    sequence: stats.sequence,
                    round: stats.round,
                    state: "pending".into(),
                    seed: key.clone(),
                    admitted: 0,
                    done: 0,
                    not_before: 0,
                }
            }
        };
        origin.admitted += 1;
        if let Some(old) = &old {
            match old.state {
                State::Committed => stats.committed_urls -= 1,
                State::Terminal => stats.terminal_urls -= 1,
                _ => anyhow::bail!("unfinished URL from an earlier frontier round"),
            }
        } else {
            stats.urls += 1;
        }
        stats.pending_urls += 1;
        put(
            tx,
            URLS,
            &key,
            &UrlRow {
                url: key.clone(),
                origin: url.origin().ascii_serialization(),
                site: site.clone(),
                round: stats.round,
                state: State::Pending,
                fetch_id: None,
                reason: None,
                not_before: 0,
                sitemap: false,
                continuation: None,
            },
        )?;
        tx.open_table(ELIGIBLE)?
            .insert((site.as_str(), key.as_str()), 0)?;
        let row: UrlRow = read(tx, URLS, &key)?.context("missing admitted URL")?;
        Self::queue_http_origin(tx, stats, &row)?;
        put(tx, ORIGINS, &site, &origin)?;
        Ok(true)
    }

    pub fn discover(&self, urls: impl IntoIterator<Item = Url>) -> Result<()> {
        self.admit_batches(urls, false)
    }

    pub fn admit_urls(&self, urls: impl IntoIterator<Item = Url>) -> Result<()> {
        self.admit_batches(urls, true)
    }

    pub fn is_sitemap(&self, url: &Url) -> Result<bool> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(URLS)?;
        let value = table.get(crate::capture::url_key(url)?.as_str())?;
        Ok(value
            .map(|value| serde_json::from_slice::<UrlRow>(value.value()).map(|row| row.sitemap))
            .transpose()?
            .unwrap_or(false))
    }

    pub fn admit_sitemaps(&self, urls: Vec<Url>) -> Result<()> {
        for batch in urls.chunks(BATCH_SIZE) {
            self.transaction(|tx, stats| {
                for url in batch {
                    let key = crate::capture::url_key(url)?;
                    if read::<UrlRow>(tx, URLS, &key)?.is_some() {
                        continue;
                    }
                    if self.admit(tx, stats, url, true)? {
                        let mut row: UrlRow =
                            read(tx, URLS, &key)?.context("missing admitted sitemap")?;
                        row.sitemap = true;
                        put(tx, URLS, &key, &row)?;
                    }
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn admit_batches(&self, urls: impl IntoIterator<Item = Url>, internal: bool) -> Result<()> {
        let mut urls = urls.into_iter();
        loop {
            let batch: Vec<_> = urls.by_ref().take(BATCH_SIZE).collect();
            if batch.is_empty() {
                break;
            }
            self.transaction(|tx, stats| {
                for url in &batch {
                    self.admit(tx, stats, url, internal)?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    pub fn begin_url(&self, url: &Url) -> Result<Option<String>> {
        self.transaction(|tx, stats| {
            self.admit(tx, stats, url, true)?;
            let key = crate::capture::url_key(url)?;
            let Some(mut row): Option<UrlRow> = read(tx, URLS, &key)? else {
                return Ok(None);
            };
            if row.round != stats.round || row.state != State::Pending {
                return Ok(None);
            }
            let id = row
                .fetch_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            row.state = State::Leased;
            row.fetch_id = Some(id.clone());
            tx.open_table(LEASES)?.insert(id.as_str(), key.as_str())?;
            tx.open_table(ELIGIBLE)?
                .remove((row.site.as_str(), key.as_str()))?;
            tx.open_table(HTTP_URLS)?
                .remove((row.origin.as_str(), key.as_str()))?;
            put(tx, URLS, &key, &row)?;
            stats.pending_urls -= 1;
            stats.leased_urls += 1;
            Ok(Some(id))
        })
    }

    pub fn redirect_continuation(&self, url: &Url) -> Result<Option<super::RedirectContinuation>> {
        let key = crate::capture::url_key(url)?;
        let tx = self.db.begin_read()?;
        let table = tx.open_table(URLS)?;
        let value = table.get(key.as_str())?;
        Ok(value
            .map(|value| {
                serde_json::from_slice::<UrlRow>(value.value()).map(|row| row.continuation)
            })
            .transpose()?
            .flatten())
    }

    pub fn save_redirect_continuation(
        &self,
        url: &Url,
        continuation: super::RedirectContinuation,
    ) -> Result<()> {
        super::identity::UrlIdentity::from_url(&continuation.url)?;
        anyhow::ensure!(
            continuation.redirects.len() <= u8::MAX as usize
                && serde_json::to_vec(&continuation)?.len() <= 2 * 1024 * 1024,
            "redirect continuation exceeds its bound"
        );
        let key = crate::capture::url_key(url)?;
        self.transaction(|tx, _| {
            let mut row: UrlRow =
                read(tx, URLS, &key)?.context("redirect has no durably admitted URL")?;
            anyhow::ensure!(
                matches!(row.state, State::Pending | State::Leased),
                "redirect URL already resolved"
            );
            row.continuation = Some(continuation);
            put(tx, URLS, &key, &row)
        })
    }

    pub fn commit_capture(
        &self,
        metadata: &crate::capture::Metadata,
        receipt: &Receipt,
        reconciled: bool,
    ) -> Result<bool> {
        metadata.validate()?;
        anyhow::ensure!(
            metadata.fetch_id == receipt.capture_id && receipt.capture_version == Some(1),
            "frontier/WARC fetch_id contract mismatch"
        );
        let changed = self.transaction(|tx, stats| {
            if let Some(previous) = read::<CommitRow>(tx, COMMITS, &metadata.fetch_id)? {
                anyhow::ensure!(
                    previous.receipt.sha256 == receipt.sha256
                        && previous.receipt.segment == receipt.segment,
                    "conflicting durable capture for fetch_id {}",
                    metadata.fetch_id
                );
                return Ok(false);
            }
            let key = crate::capture::url_key(&Url::parse(&metadata.requested_url)?)?;
            let mut row: UrlRow =
                read(tx, URLS, &key)?.context("capture has no durably admitted URL")?;
            anyhow::ensure!(
                row.fetch_id.as_deref() == Some(&metadata.fetch_id),
                "capture has no matching URL lease"
            );
            match row.state {
                State::Leased => stats.leased_urls -= 1,
                State::Pending => stats.pending_urls -= 1,
                _ => anyhow::bail!("URL already resolved by another capture"),
            }
            row.state = State::Committed;
            row.continuation = None;
            put(tx, URLS, &key, &row)?;
            tx.open_table(LEASES)?.remove(metadata.fetch_id.as_str())?;
            tx.open_table(ELIGIBLE)?
                .remove((row.site.as_str(), key.as_str()))?;
            tx.open_table(HTTP_URLS)?
                .remove((row.origin.as_str(), key.as_str()))?;
            Self::reschedule_http_origin(tx, stats, &row.origin, row.not_before)?;
            put(
                tx,
                COMMITS,
                &metadata.fetch_id,
                &CommitRow {
                    url: key,
                    receipt: receipt.clone(),
                },
            )?;
            put(
                tx,
                OUTBOX,
                &metadata.fetch_id,
                &Discovery {
                    internal: metadata.internal_urls.clone(),
                    external: metadata.discovery_urls.clone(),
                },
            )?;
            if let Some(mut origin) = read::<OriginRow>(tx, ORIGINS, &row.site)? {
                origin.done += 1;
                put(tx, ORIGINS, &row.site, &origin)?;
            }
            stats.committed_urls += 1;
            stats.commits += 1;
            stats.outbox_entries += 1;
            if reconciled {
                stats.reconciled_captures += 1;
            }
            Ok(true)
        })?;
        Ok(changed)
    }

    pub fn drain_outbox(&self) -> Result<()> {
        loop {
            let id = {
                let read = self.db.begin_read()?;
                let table = read.open_table(OUTBOX)?;
                let first = table.iter()?.next().transpose()?;
                first.map(|(key, _)| key.value().to_owned())
            };
            let Some(id) = id else {
                break;
            };
            self.transaction(|tx, stats| {
                let Some(entry): Option<Discovery> = read(tx, OUTBOX, &id)? else {
                    return Ok(());
                };
                for url in &entry.internal {
                    self.admit(tx, stats, url, true)?;
                }
                for url in &entry.external {
                    self.admit(tx, stats, url, false)?;
                }
                tx.open_table(OUTBOX)?.remove(id.as_str())?;
                stats.outbox_entries -= 1;
                Ok(())
            })?;
        }
        Ok(())
    }

    pub fn claim(&self) -> Result<Option<Job>> {
        self.claim_at(chrono::Utc::now())
    }

    pub fn claim_at(&self, now: chrono::DateTime<chrono::Utc>) -> Result<Option<Job>> {
        self.claim_limited_at(now, BATCH_SIZE, false)
    }

    pub fn next_eligible_at(&self) -> Result<Option<u64>> {
        let tx = self.db.begin_read()?;
        let ready = tx.open_table(READY)?;
        let next = ready
            .iter()?
            .next()
            .transpose()?
            .map(|(key, _)| key.value().0);
        Ok(next)
    }

    pub fn claim_turn_at(&self, now: chrono::DateTime<chrono::Utc>) -> Result<Option<Job>> {
        self.claim_limited_at(now, 1, true)
    }

    fn claim_limited_at(
        &self,
        now: chrono::DateTime<chrono::Utc>,
        limit: usize,
        turn: bool,
    ) -> Result<Option<Job>> {
        if self.next_eligible_at()?.is_none_or(|due| due > millis(now)) {
            return Ok(None);
        }
        self.transaction(|tx, stats| {
            let next = {
                let ready = tx.open_table(READY)?;
                let first = ready.iter()?.next().transpose()?;
                first.map(|(key, value)| (key.value().0, value.value().to_owned()))
            };
            let Some((due, site)) = next else {
                return Ok(None);
            };
            if due > millis(now) {
                return Ok(None);
            }
            let mut origin: OriginRow =
                read(tx, ORIGINS, &site)?.context("missing scheduled origin")?;
            let pending: Vec<String> = if turn {
                let (due, http_origin) = Self::next_http_origin(tx, &site)?
                    .context("ready site has no queued HTTP origin")?;
                if due > millis(now) {
                    anyhow::bail!("site eligibility precedes its HTTP origins");
                }
                let pending = tx
                    .open_table(HTTP_URLS)?
                    .range((http_origin.as_str(), "")..=(http_origin.as_str(), "\u{10ffff}"))?
                    .take(1)
                    .map(|entry| Ok(entry?.0.value().1.to_owned()))
                    .collect::<Result<Vec<_>>>()?;
                let row: HttpOriginRow =
                    read(tx, HTTP_ORIGINS, &http_origin)?.context("missing queued HTTP origin")?;
                tx.open_table(HTTP_READY)?
                    .remove((site.as_str(), row.not_before, row.sequence))?;
                tx.open_table(HTTP_ACTIVE)?
                    .insert(site.as_str(), http_origin.as_str())?;
                pending
            } else {
                let table = tx.open_table(ELIGIBLE)?;
                table
                    .range((site.as_str(), "")..=(site.as_str(), "\u{10ffff}"))?
                    .take(limit)
                    .map(|entry| {
                        let (key, _) = entry?;
                        Ok(key.value().1.to_owned())
                    })
                    .collect::<Result<_>>()?
            };
            let mut urls = VecDeque::new();
            for key in pending {
                if let Some(row) = read::<UrlRow>(tx, URLS, &key)? {
                    if row.round == stats.round && row.state == State::Pending {
                        urls.push_back(WeightedUrl {
                            url: Url::parse(&key)?,
                            weight: 1.0,
                        });
                    }
                }
            }
            if urls.is_empty() && origin.done == 0 {
                urls.push_back(WeightedUrl {
                    url: Url::parse(&origin.seed)?,
                    weight: 1.0,
                });
            }
            tx.open_table(READY)?.remove((due, origin.sequence))?;
            origin.state = "active".into();
            put(tx, ORIGINS, &site, &origin)?;
            tx.open_table(ACTIVE)?.insert(site.as_str(), ())?;
            stats.pending_sites -= 1;
            stats.active_sites += 1;
            Ok(Some(Job {
                domain: Domain::from(Url::parse(&origin.seed)?),
                wandering_urls: if turn {
                    0
                } else {
                    self.budget
                        .saturating_sub(origin.done)
                        .saturating_sub(urls.len() as u64)
                },
                urls,
            }))
        })
    }

    pub fn resolve(
        &self,
        domain: &Domain,
        outcome: &JobOutcome,
        min_delay_ms: u64,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        self.resolve_mode(domain, outcome, min_delay_ms, now, false)
    }

    pub fn resolve_turn(
        &self,
        domain: &Domain,
        outcome: &JobOutcome,
        min_delay_ms: u64,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<()> {
        self.resolve_mode(domain, outcome, min_delay_ms, now, true)
    }

    fn resolve_mode(
        &self,
        domain: &Domain,
        outcome: &JobOutcome,
        min_delay_ms: u64,
        now: chrono::DateTime<chrono::Utc>,
        turn: bool,
    ) -> Result<()> {
        if outcome.status == JobStatus::StorageFailed {
            anyhow::bail!("Crawler storage failed; URL leases remain recoverable");
        }
        let status = self.transaction(|tx, stats| {
            let site = domain.as_str();
            let _: OriginRow = read(tx, ORIGINS, site)?.context("missing active crawler origin")?;
            let mut delay = min_delay_ms;
            let mut completed = 0;
            for entry in &outcome.urls {
                let key = crate::capture::url_key(&entry.url)?;
                if read::<UrlRow>(tx, URLS, &key)?.is_none() {
                    self.admit(tx, stats, &entry.url, true)?;
                }
                let Some(mut row): Option<UrlRow> = read(tx, URLS, &key)? else {
                    continue;
                };
                if matches!(row.state, State::Committed | State::Terminal) {
                    continue;
                }
                if row.state == State::Leased {
                    stats.leased_urls -= 1;
                    if let Some(id) = &row.fetch_id {
                        tx.open_table(LEASES)?.remove(id.as_str())?;
                    }
                } else {
                    stats.pending_urls -= 1;
                }
                match &entry.outcome {
                    FetchOutcome::Deferred {
                        retry_after_ms,
                        reason,
                    } => {
                        delay = delay.max(retry_after_ms.unwrap_or(0));
                        row.state = State::Pending;
                        row.reason = Some(reason.clone());
                        stats.pending_urls += 1;
                    }
                    FetchOutcome::Cancelled => {
                        row.state = State::Pending;
                        stats.pending_urls += 1;
                    }
                    other => {
                        row.state = State::Terminal;
                        row.continuation = None;
                        row.reason = Some(format!("{other:?}"));
                        stats.terminal_urls += 1;
                        completed += 1;
                    }
                }
                if row.state == State::Pending {
                    row.not_before = millis(now).saturating_add(delay);
                    tx.open_table(ELIGIBLE)?
                        .insert((site, key.as_str()), row.not_before)?;
                    Self::queue_http_origin(tx, stats, &row)?;
                } else {
                    tx.open_table(ELIGIBLE)?.remove((site, key.as_str()))?;
                    tx.open_table(HTTP_URLS)?
                        .remove((row.origin.as_str(), key.as_str()))?;
                    Self::reschedule_http_origin(tx, stats, &row.origin, row.not_before)?;
                }
                put(tx, URLS, &key, &row)?;
            }
            let mut origin: OriginRow =
                read(tx, ORIGINS, site)?.context("missing resolved origin")?;
            origin.done += completed;
            let remaining = {
                let table = tx.open_table(ELIGIBLE)?;
                let first = table.range((site, "")..=(site, "\u{10ffff}"))?.next();
                first.is_some()
            };
            stats.active_sites = stats.active_sites.saturating_sub(1);
            tx.open_table(ACTIVE)?.remove(site)?;
            let http_origin = tx
                .open_table(HTTP_ACTIVE)?
                .remove(site)?
                .map(|v| v.value().to_owned());
            if let Some(key) = http_origin {
                Self::reschedule_http_origin(tx, stats, &key, millis(now).saturating_add(delay))?;
            }
            if outcome.status == JobStatus::Completed && !remaining {
                origin.state = "done".into();
            } else {
                origin.state = "pending".into();
                origin.not_before = millis(now).saturating_add(delay);
                if turn {
                    origin.not_before =
                        Self::next_http_origin(tx, site)?.map_or(origin.not_before, |(due, _)| due);
                }
                stats.sequence += 1;
                origin.sequence = stats.sequence;
                tx.open_table(READY)?
                    .insert((origin.not_before, origin.sequence), site)?;
                stats.pending_sites += 1;
            }
            put(tx, ORIGINS, site, &origin)?;
            Ok((origin.state, origin.not_before))
        })?;
        if let Some(observer) = &self.observer {
            if status.0 == "done" {
                observer.event(domain, None, "site_completed", "Cola del sitio resuelta");
            } else {
                observer.event(
                    domain,
                    None,
                    "site_deferred",
                    "Turno cerrado; trabajador disponible para otro origen",
                );
                observer.waiting(
                    domain,
                    std::time::Duration::from_millis(status.1.saturating_sub(millis(now))),
                );
            }
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        let s = self.metrics.snapshot();
        s.pending_sites == 0 && s.active_sites == 0 && s.outbox_entries == 0
    }

    pub fn restart_round(&self, seeds: Vec<Url>) -> Result<()> {
        anyhow::ensure!(self.is_empty(), "Cannot reset active frontier");
        self.transaction(|_, stats| {
            stats.round += 1;
            stats.known_sites = 0;
            Ok(())
        })?;
        self.discover(seeds)
    }

    fn recover_leases(&self) -> Result<()> {
        loop {
            let batch: Vec<_> = {
                let read = self.db.begin_read()?;
                let table = read.open_table(LEASES)?;
                table
                    .iter()?
                    .take(BATCH_SIZE)
                    .map(|entry| {
                        let (id, url) = entry?;
                        Ok((id.value().to_owned(), url.value().to_owned()))
                    })
                    .collect::<Result<_>>()?
            };
            if batch.is_empty() {
                break;
            }
            self.transaction(|tx, stats| {
                for (id, key) in &batch {
                    let mut row: UrlRow = read(tx, URLS, key)?.context("orphan URL lease")?;
                    if row.state == State::Leased {
                        row.state = State::Pending;
                        stats.leased_urls -= 1;
                        stats.pending_urls += 1;
                        stats.recovered_leases += 1;
                        tx.open_table(ELIGIBLE)?
                            .insert((row.site.as_str(), key.as_str()), row.not_before)?;
                        Self::queue_http_origin(tx, stats, &row)?;
                        put(tx, URLS, key, &row)?;
                    }
                    tx.open_table(LEASES)?.remove(id.as_str())?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn recover_jobs(&self) -> Result<()> {
        loop {
            let sites: Vec<String> = {
                let read = self.db.begin_read()?;
                let table = read.open_table(ACTIVE)?;
                table
                    .iter()?
                    .take(BATCH_SIZE)
                    .map(|entry| Ok(entry?.0.value().to_owned()))
                    .collect::<Result<_>>()?
            };
            if sites.is_empty() {
                break;
            }
            self.transaction(|tx, stats| {
                for site in &sites {
                    let mut row: OriginRow =
                        read(tx, ORIGINS, site)?.context("orphan origin lease")?;
                    if row.state == "active" {
                        let http_origin = tx
                            .open_table(HTTP_ACTIVE)?
                            .remove(site.as_str())?
                            .map(|v| v.value().to_owned());
                        if let Some(key) = http_origin {
                            Self::reschedule_http_origin(tx, stats, &key, row.not_before)?;
                        }
                        row.state = "pending".into();
                        put(tx, ORIGINS, site, &row)?;
                        tx.open_table(READY)?
                            .insert((row.not_before, row.sequence), site.as_str())?;
                        stats.active_sites -= 1;
                        stats.pending_sites += 1;
                    }
                    tx.open_table(ACTIVE)?.remove(site.as_str())?;
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    pub fn has_commit(&self, id: &str) -> Result<bool> {
        let read = self.db.begin_read()?;
        Ok(read.open_table(COMMITS)?.get(id)?.is_some())
    }

    pub fn reconcile_spool(&self, directory: &Path) -> Result<()> {
        use std::io::{BufRead, Read, Seek};
        #[derive(Deserialize)]
        struct Journal {
            version: u32,
            receipts: Vec<Receipt>,
            sha256: String,
        }
        let segments = directory.join("segments");
        if !segments.exists() {
            return Ok(());
        }
        for entry in fs::read_dir(&segments)? {
            let path = entry?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("commit") {
                continue;
            }
            let mut journal = std::io::BufReader::new(fs::File::open(&path)?);
            loop {
                let mut line = Vec::new();
                let size = journal
                    .by_ref()
                    .take(1024 * 1024 + 1)
                    .read_until(b'\n', &mut line)?;
                if size == 0 {
                    break;
                }
                anyhow::ensure!(
                    size <= 1024 * 1024 && line.last() == Some(&b'\n'),
                    "invalid durable capture journal"
                );
                let group: Journal = serde_json::from_slice(&line)?;
                anyhow::ensure!(
                    group.version == 1
                        && group.receipts.len() <= 256
                        && crate::capture::sha256(&serde_json::to_vec(&group.receipts)?)
                            == group.sha256,
                    "invalid capture journal checksum/version"
                );
                for receipt in &group.receipts {
                    if receipt.capture_version.is_none() || self.has_commit(&receipt.capture_id)? {
                        continue;
                    }
                    anyhow::ensure!(
                        receipt.capture_version == Some(1)
                            && receipt.end > receipt.start
                            && receipt.end - receipt.start <= 256 * 1024 * 1024,
                        "unsupported capture receipt version or range"
                    );
                    anyhow::ensure!(
                        Path::new(&receipt.segment)
                            .file_name()
                            .and_then(|s| s.to_str())
                            == Some(receipt.segment.as_str()),
                        "invalid archive reference"
                    );
                    let mut data = fs::File::open(segments.join(&receipt.segment))?;
                    data.seek(std::io::SeekFrom::Start(receipt.start))?;
                    let range = data.take(receipt.end - receipt.start);
                    let mut records = crate::warc::RecordIterator::new(range);
                    let record = records.next().context("durable receipt has no capture")??;
                    let capture = record
                        .metadata
                        .capture
                        .context("receipt references legacy capture")?;
                    self.commit_capture(&capture, receipt, true)?;
                }
            }
        }
        self.drain_outbox()
    }

    #[cfg(test)]
    pub(super) fn complete(&self, domain: &Domain) -> Result<()> {
        let site = domain.as_str();
        let mut outcome = JobOutcome::default();
        let urls: Vec<_> = {
            let read = self.db.begin_read()?;
            let table = read.open_table(ELIGIBLE)?;
            table
                .range((site, "")..=(site, "\u{10ffff}"))?
                .map(|entry| Ok(Url::parse(entry?.0.value().1)?))
                .collect::<Result<_>>()?
        };
        for url in urls {
            outcome.record(
                url,
                FetchOutcome::Skipped {
                    reason: "legacy completed job".into(),
                },
            );
        }
        self.resolve(domain, &outcome, 0, chrono::Utc::now())
    }

    fn migrate(&self, path: &Path) -> Result<()> {
        if path.exists() {
            let backup = path.with_extension("json.pre-c10");
            if !backup.exists() {
                let temporary = backup.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
                let mut input = fs::File::open(path)?;
                let mut output = fs::OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temporary)?;
                std::io::copy(&mut input, &mut output)?;
                output.sync_all()?;
                fs::rename(&temporary, &backup)?;
                if let Some(parent) = path.parent() {
                    fs::File::open(parent)?.sync_all()?;
                }
            }
            self.migrate_stream(path)?;
        }
        self.transaction(|_, stats| {
            stats.migration_complete = true;
            Ok(())
        })
    }

    fn migrate_job(&self, job: Job, not_before: u64) -> Result<()> {
        let sites: std::collections::BTreeSet<String> = job
            .urls
            .iter()
            .map(|url| Domain::from(&url.url).as_str().to_owned())
            .collect();
        self.transaction(|tx, stats| {
            for site in &sites {
                if let Some(mut row) = read::<OriginRow>(tx, ORIGINS, site)? {
                    if row.state == "done" && row.admitted == 0 {
                        row.state = "pending".into();
                        row.seed = job
                            .urls
                            .iter()
                            .find(|url| Domain::from(&url.url).as_str() == site)
                            .context("legacy job has no seed")?
                            .url
                            .to_string();
                        tx.open_table(READY)?
                            .insert((not_before, row.sequence), site.as_str())?;
                        put(tx, ORIGINS, site, &row)?;
                        stats.pending_sites += 1;
                    }
                }
            }
            Ok(())
        })?;
        self.admit_urls(job.urls.iter().map(|url| url.url.clone()))?;
        self.transaction(|tx, stats| {
            for site in &sites {
                if let Some(mut row) = read::<OriginRow>(tx, ORIGINS, site)? {
                    tx.open_table(READY)?
                        .remove((row.not_before, row.sequence))?;
                    row.not_before = not_before;
                    tx.open_table(READY)?
                        .insert((not_before, row.sequence), site.as_str())?;
                    put(tx, ORIGINS, site, &row)?;
                }
            }
            stats.migrated_jobs += 1;
            Ok(())
        })
    }

    fn migrate_seen(&self, site: &str) -> Result<()> {
        self.transaction(|tx, stats| {
            if read::<OriginRow>(tx, ORIGINS, site)?.is_none() {
                stats.sequence += 1;
                put(
                    tx,
                    ORIGINS,
                    site,
                    &OriginRow {
                        sequence: stats.sequence,
                        round: stats.round,
                        state: "done".into(),
                        seed: format!("https://{site}/"),
                        admitted: 0,
                        done: 0,
                        not_before: 0,
                    },
                )?;
                stats.known_sites += 1;
            }
            Ok(())
        })
    }

    fn migrate_stream(&self, path: &Path) -> Result<()> {
        use serde::de::{DeserializeSeed, MapAccess, SeqAccess, Visitor};
        struct Root<'a>(&'a Frontier);
        struct Entries<'a> {
            frontier: &'a Frontier,
            kind: String,
        }
        impl<'de> DeserializeSeed<'de> for Entries<'_> {
            type Value = ();
            fn deserialize<D: serde::Deserializer<'de>>(
                self,
                d: D,
            ) -> std::result::Result<(), D::Error> {
                if self.kind == "active" {
                    d.deserialize_map(self)
                } else {
                    d.deserialize_seq(self)
                }
            }
        }
        impl<'de> Visitor<'de> for Entries<'_> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("legacy frontier entries")
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> std::result::Result<(), A::Error> {
                match self.kind.as_str() {
                    "seen" => {
                        while let Some(site) = seq.next_element::<String>()? {
                            self.frontier
                                .migrate_seen(&site)
                                .map_err(serde::de::Error::custom)?;
                        }
                    }
                    "delayed" => {
                        while let Some(job) = seq.next_element::<super::local::DelayedJob>()? {
                            self.frontier
                                .migrate_job(job.job, millis(job.not_before))
                                .map_err(serde::de::Error::custom)?;
                        }
                    }
                    _ => {
                        while let Some(job) = seq.next_element::<Job>()? {
                            self.frontier
                                .migrate_job(job, 0)
                                .map_err(serde::de::Error::custom)?;
                        }
                    }
                }
                Ok(())
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
                while let Some((_, job)) = map.next_entry::<String, Job>()? {
                    self.frontier
                        .migrate_job(job, 0)
                        .map_err(serde::de::Error::custom)?;
                }
                Ok(())
            }
        }
        impl<'de> Visitor<'de> for Root<'_> {
            type Value = ();
            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("legacy frontier JSON")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> std::result::Result<(), A::Error> {
                while let Some(kind) = map.next_key::<String>()? {
                    if matches!(kind.as_str(), "pending" | "active" | "delayed" | "seen") {
                        map.next_value_seed(Entries {
                            frontier: self.0,
                            kind,
                        })?;
                    } else {
                        let _: serde::de::IgnoredAny = map.next_value()?;
                    }
                }
                Ok(())
            }
        }
        let mut deserializer = serde_json::Deserializer::from_reader(fs::File::open(path)?);
        serde::de::Deserializer::deserialize_map(&mut deserializer, Root(self))?;
        deserializer.end()?;
        Ok(())
    }
}
