use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use anyhow::{Context, Result};
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition, WriteTransaction};
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
const CANDIDATE_SOURCE: TableDefinition<(&str, &str), ()> =
    TableDefinition::new("discovery_sources.v1");
const CANDIDATE_READY: TableDefinition<(&str, &str), ()> =
    TableDefinition::new("discovery_pending.v1");
const CANDIDATES: TableDefinition<&str, &[u8]> = TableDefinition::new("discovery_candidates.v1");
const DISCOVERY_CYCLES: TableDefinition<&str, &[u8]> = TableDefinition::new("discovery_cycles.v1");
const DOCUMENTS: TableDefinition<&str, u64> = TableDefinition::new("discovery_documents.v1");
const FAMILIES: TableDefinition<&str, &[u8]> = TableDefinition::new("trap_families.v1");
const QUARANTINE: TableDefinition<&str, &[u8]> = TableDefinition::new("quarantine.v1");
const RELEASES: TableDefinition<&str, ()> = TableDefinition::new("quarantine_releases.v1");
const RECRAWL: TableDefinition<&str, &[u8]> = TableDefinition::new("resource_versions.v1");
const RECRAWL_READY: TableDefinition<(u64, &str), ()> =
    TableDefinition::new("recrawl_eligibility.v1");
const CONTENT: TableDefinition<&str, &str> = TableDefinition::new("content_versions.v1");
const LIVE_PENDING: TableDefinition<(&str, &str), u64> =
    TableDefinition::new("live_discovery_pending.v1");
const METADATA_VERSIONS: TableDefinition<&str, ()> =
    TableDefinition::new("capture_metadata_versions.v1");
const RETRIES: TableDefinition<&str, &[u8]> = TableDefinition::new("origin_retries.v1");

fn legacy_site_version() -> u32 {
    1
}
pub const CACHE_BYTES: usize = 16 * 1024 * 1024;
pub const BATCH_SIZE: usize = 128;

#[derive(Default, Clone, Debug, Serialize, Deserialize)]
pub struct Stats {
    #[serde(default)]
    pub retry_deferred: u64,
    #[serde(default)]
    pub discovery_candidates: u64,
    #[serde(default)]
    pub discovery_documents: u64,
    #[serde(default)]
    pub discovery_bytes: u64,
    #[serde(default)]
    pub discovery_quota_stops: u64,
    #[serde(default)]
    pub discovery_invalid_xml: u64,
    #[serde(default)]
    pub quarantined_urls: u64,
    #[serde(default)]
    pub quarantine_reasons: std::collections::BTreeMap<String, u64>,
    #[serde(default)]
    pub quarantine_released: u64,
    #[serde(default)]
    pub quarantine_recent: Vec<super::traps::Quarantine>,
    #[serde(default)]
    pub review_samples: u64,
    #[serde(default)]
    pub duplicate_content: u64,
    #[serde(default)]
    pub content_versions: u64,
    #[serde(default)]
    pub empty_pages: u64,
    #[serde(default)]
    pub retry_exhausted: u64,
    #[serde(default)]
    pub circuits_opened: u64,
    #[serde(default)]
    pub retry_recovered: u64,
    #[serde(default)]
    pub revalidations: u64,
    #[serde(default)]
    pub recrawl_changes: u64,
    #[serde(default)]
    pub recrawl_bytes_saved: u64,
    #[serde(default)]
    pub http_attempts: u64,
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
    #[serde(default = "default_url_weight")]
    weight: f64,
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
    retry: super::retry::UrlState,
    #[serde(default)]
    sitemap_depth: u32,
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
    retry_config: Mutex<super::retry::Config>,
    discovery_config: Mutex<super::discovery::Config>,
    traps_config: Mutex<super::traps::Config>,
    recrawl_config: Mutex<super::recrawl::Config>,
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

fn default_url_weight() -> f64 {
    1.0
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
        tx.open_table(RETRIES)?;
        tx.open_table(LIVE_PENDING)?;
        tx.open_table(METADATA_VERSIONS)?;
        tx.open_table(CANDIDATES)?;
        tx.open_table(CANDIDATE_READY)?;
        tx.open_table(CANDIDATE_SOURCE)?;
        tx.open_table(DISCOVERY_CYCLES)?;
        tx.open_table(DOCUMENTS)?;
        tx.open_table(FAMILIES)?;
        tx.open_table(QUARANTINE)?;
        tx.open_table(RELEASES)?;
        tx.open_table(CONTENT)?;
        tx.open_table(RECRAWL)?;
        tx.open_table(RECRAWL_READY)?;
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
            retry_config: Mutex::new(Default::default()),
            discovery_config: Mutex::new(Default::default()),
            traps_config: Mutex::new(Default::default()),
            recrawl_config: Mutex::new(Default::default()),
        };
        frontier.migrate_site_identity()?;
        frontier.migrate_http_origins()?;
        if !frontier.metrics.snapshot().migration_complete {
            if legacy_path != frontier.path {
                frontier.migrate(&legacy_path)?;
            } else {
                frontier.transaction(|_, stats| {
                    stats.migration_complete = true;
                    Ok(())
                })?;
            }
        }
        frontier.recover_leases()?;
        frontier.recover_jobs()?;
        frontier.repair_ready_indexes()?;
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
        origin.not_before = due.max(
            read::<super::retry::OriginState>(tx, RETRIES, key)?.map_or(0, |r| r.not_before_ms),
        );
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
                .insert((origin.site.as_str(), origin.not_before, origin.sequence), key)?;
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

    fn repair_ready_indexes(&self) -> Result<()> {
        let version = {
            let tx = self.db.begin_read()?;
            let table = tx.open_table(META)?;
            let value = table.get("ready-index-version")?;
            value.map(|v| serde_json::from_slice::<u32>(v.value())).transpose()?.unwrap_or(1)
        };
        anyhow::ensure!(version <= 2, "unsupported frontier readiness index version");
        if version == 2 { return Ok(()); }
        self.transaction(|tx, _| {
            if read::<String>(tx, META, "ready-repair-phase")?.is_none() {
                // Derived indexes only: URL identities, leases, capture commits,
                // retry deadlines and outbox remain the durable authorities.
                tx.delete_table(HTTP_READY)?;
                tx.delete_table(READY)?;
                tx.open_table(HTTP_READY)?;
                tx.open_table(READY)?;
                put(tx, META, "ready-repair-phase", &"http")?;
                put(tx, META, "ready-repair-cursor", &String::new())?;
            }
            Ok(())
        })?;
        loop {
            let finished = self.transaction(|tx, stats| {
                let phase: String = read(tx, META, "ready-repair-phase")?.context("missing readiness repair phase")?;
                anyhow::ensure!(phase == "http" || phase == "sites", "unsupported readiness repair phase");
                let cursor: String = read(tx, META, "ready-repair-cursor")?.unwrap_or_default();
                let table = if phase == "http" { HTTP_ORIGINS } else { ORIGINS };
                let keys = tx.open_table(table)?.range::<&str>((
                    std::ops::Bound::Excluded(cursor.as_str()), std::ops::Bound::Unbounded,
                ))?.take(BATCH_SIZE).map(|entry| Ok(entry?.0.value().to_owned())).collect::<Result<Vec<_>>>()?;
                for key in &keys {
                    if phase == "http" {
                        let mut row: HttpOriginRow = read(tx, HTTP_ORIGINS, key)?.context("missing HTTP origin during readiness repair")?;
                        if row.round != stats.round { continue; }
                        let remaining = tx.open_table(HTTP_URLS)?.range((key.as_str(), "")..=(key.as_str(), "\u{10ffff}"))?.next().transpose()?.is_some();
                        if !remaining { continue; }
                        row.not_before = row.not_before.max(read::<super::retry::OriginState>(tx, RETRIES, key)?.map_or(0, |retry| retry.not_before_ms));
                        tx.open_table(HTTP_READY)?.insert((row.site.as_str(), row.not_before, row.sequence), key.as_str())?;
                        put(tx, HTTP_ORIGINS, key, &row)?;
                    } else {
                        let mut row: OriginRow = read(tx, ORIGINS, key)?.context("missing site during readiness repair")?;
                        if row.round != stats.round || row.state != "pending" { continue; }
                        row.not_before = Self::next_http_origin(tx, key)?.map_or(row.not_before, |(due, _)| due);
                        tx.open_table(READY)?.insert((row.not_before, row.sequence), key.as_str())?;
                        put(tx, ORIGINS, key, &row)?;
                    }
                }
                if let Some(last) = keys.last() {
                    put(tx, META, "ready-repair-cursor", last)?;
                } else if phase == "http" {
                    put(tx, META, "ready-repair-phase", &"sites")?;
                    put(tx, META, "ready-repair-cursor", &String::new())?;
                } else {
                    put(tx, META, "ready-index-version", &2u32)?;
                    tx.open_table(META)?.remove("ready-repair-phase")?;
                    tx.open_table(META)?.remove("ready-repair-cursor")?;
                    return Ok(true);
                }
                Ok(false)
            })?;
            if finished { return Ok(()); }
        }
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

    pub fn configure_discovery(
        &self,
        discovery: &super::discovery::Config,
        traps: &super::traps::Config,
    ) -> Result<()> {
        discovery.validate()?;
        traps.validate()?;
        *self.discovery_config.lock().unwrap() = discovery.clone();
        *self.traps_config.lock().unwrap() = traps.clone();
        Ok(())
    }
    fn trap_admission(&self, tx: &WriteTransaction, stats: &mut Stats, url: &Url) -> Result<bool> {
        let config = self.traps_config.lock().unwrap().clone();
        if !config.enabled {
            return Ok(true);
        }
        let key = crate::capture::url_key(url)?;
        if tx.open_table(RELEASES)?.get(key.as_str())?.is_some() {
            return Ok(true);
        }
        if read::<super::traps::Quarantine>(tx, QUARANTINE, &key)?.is_some() {
            return Ok(false);
        }
        let (pattern, policy) = config.pattern(url);
        let mut family: super::traps::Family = read(tx, FAMILIES, &pattern)?.unwrap_or_default();
        let reason = policy
            .filter(|(_, limit)| family.admitted >= *limit)
            .map(|(reason, _)| reason)
            .or_else(|| {
                (family.repeated >= config.repeated_content_budget)
                    .then_some("repeated_content_family")
            })
            .or_else(|| {
                (family.empty >= config.repeated_content_budget).then_some("empty_content_family")
            });
        if let Some(reason) = reason {
            let review_sample = stats.quarantined_urls % config.review_sample_every == 0;
            let row = super::traps::Quarantine {
                url: url.clone(),
                reason: reason.into(),
                pattern,
                created_at: chrono::Utc::now(),
                review_sample,
            };
            put(tx, QUARANTINE, &key, &row)?;
            if stats.quarantine_recent.len() >= 8 {
                stats.quarantine_recent.remove(0);
            }
            stats.quarantine_recent.push(row);
            stats.quarantined_urls += 1;
            *stats.quarantine_reasons.entry(reason.into()).or_default() += 1;
            stats.review_samples += u64::from(review_sample);
            return Ok(false);
        }
        family.admitted += 1;
        put(tx, FAMILIES, &pattern, &family)?;
        Ok(true)
    }
    pub fn quarantine(&self, limit: usize) -> Result<Vec<super::traps::Quarantine>> {
        self.db
            .begin_read()?
            .open_table(QUARANTINE)?
            .iter()?
            .take(limit.min(128))
            .map(|row| {
                let (_, value) = row?;
                Ok(serde_json::from_slice(value.value())?)
            })
            .collect()
    }
    pub fn release_quarantine(&self, url: &Url) -> Result<bool> {
        let key = crate::capture::url_key(url)?;
        self.transaction(|tx, stats| {
            if tx.open_table(QUARANTINE)?.remove(key.as_str())?.is_some() {
                tx.open_table(RELEASES)?.insert(key.as_str(), ())?;
                stats.quarantine_released += 1;
                let site = Domain::from(url).as_str().to_string();
                if let Some(mut origin) = read::<OriginRow>(tx, ORIGINS, &site)? {
                    if origin.state == "done" {
                        origin.state = "pending".into();
                        origin.not_before = 0;
                        stats.sequence += 1;
                        origin.sequence = stats.sequence;
                        tx.open_table(READY)?
                            .insert((0, origin.sequence), site.as_str())?;
                        stats.pending_sites += 1;
                        put(tx, ORIGINS, &site, &origin)?;
                    }
                    origin.admitted = origin.admitted.saturating_sub(1);
                    put(tx, ORIGINS, &site, &origin)?;
                }
                self.admit(tx, stats, url, true)?;
                stats.quarantine_recent.retain(|row| row.url != *url);
                return Ok(true);
            }
            Ok(false)
        })
    }
    pub fn promote_candidates(&self) -> Result<usize> {
        let mut cursor: Option<(String, String)> = {
            let tx = self.db.begin_read()?;
            tx.open_table(META)?
                .get("candidate-promotion-cursor")?
                .map(|v| serde_json::from_slice::<Option<(String, String)>>(v.value()))
                .transpose()?
                .flatten()
        };
        let mut promoted = 0;
        for _ in 0..8 {
            let batch = {
                let tx = self.db.begin_read()?;
                let table = tx.open_table(CANDIDATE_READY)?;
                let (site, key) = cursor.clone().unwrap_or_default();
                table
                    .range((site.as_str(), key.as_str())..)?
                    .filter_map(|r| match r {
                        Ok((k, _)) => {
                            let (site, key) = k.value();
                            if cursor.as_ref().is_some_and(|c| c.0 == site && c.1 == key) {
                                None
                            } else {
                                Some(Ok((site.to_string(), key.to_string())))
                            }
                        }
                        Err(e) => Some(Err(e)),
                    })
                    .take(BATCH_SIZE)
                    .collect::<std::result::Result<Vec<_>, _>>()?
            };
            if batch.is_empty() {
                if cursor.is_none() {
                    return Ok(promoted);
                }
                cursor = None;
                break;
            }
            promoted += self.transaction(|tx, stats| {
                let mut count = 0;
                for (site, key) in &batch {
                    if read::<OriginRow>(tx, ORIGINS, site)?
                        .is_some_and(|r| r.round == stats.round && r.admitted >= self.budget)
                    {
                        continue;
                    }
                    let candidate: super::discovery::Candidate = read(tx, CANDIDATES, key)?
                        .context("pending discovery without candidate")?;
                    if self.admit(tx, stats, &candidate.url, true)? {
                        let mut row: UrlRow = read(tx, URLS, key)?.unwrap();
                        row.sitemap = candidate.sitemap;
                        row.sitemap_depth = candidate.depth;
                        if let Some(seed) =
                            read::<serde_json::Value>(tx, META, &format!("seed:{key}"))?
                        {
                            row.weight = seed["weight"]
                                .as_f64()
                                .filter(|n| n.is_finite() && *n >= 0.0)
                                .unwrap_or(1.0);
                        }
                        put(tx, URLS, key, &row)?;
                        count += 1;
                    }
                    if read::<UrlRow>(tx, URLS, key)?.is_some_and(|r| r.round == stats.round) {
                        tx.open_table(CANDIDATE_READY)?
                            .remove((site.as_str(), key.as_str()))?;
                    }
                }
                Ok(count)
            })?;
            cursor = batch.last().cloned();
            if let Some((site, key)) = &mut cursor {
                let tx = self.db.begin_read()?;
                let blocked = tx
                    .open_table(ORIGINS)?
                    .get(site.as_str())?
                    .map(|v| serde_json::from_slice::<OriginRow>(v.value()))
                    .transpose()?
                    .is_some_and(|r| {
                        r.round == self.metrics.snapshot().round && r.admitted >= self.budget
                    });
                if blocked {
                    *key = "\u{10ffff}".into();
                }
            }
        }
        self.transaction(|tx, _| put(tx, META, "candidate-promotion-cursor", &cursor))?;
        Ok(promoted)
    }
    pub fn import_plan(&self, path: &Path) -> Result<u64> {
        use std::io::{BufRead, Read};
        let mut count = 0;
        let mut batch = Vec::with_capacity(BATCH_SIZE);
        let mut reader = std::io::BufReader::new(fs::File::open(path)?);
        loop {
            let mut line = Vec::new();
            let n = reader.by_ref().take(16385).read_until(b'\n', &mut line)?;
            if n == 0 {
                break;
            }
            anyhow::ensure!(n <= 16384, "planner seed record exceeds limit");
            let value: serde_json::Value = serde_json::from_slice(&line)?;
            anyhow::ensure!(
                value["schema_version"] == 1,
                "unsupported planner seed version"
            );
            let url = Url::parse(value["url"].as_str().context("planner seed without URL")?)?;
            let source = value["source"]
                .as_str()
                .context("planner seed without provenance")?;
            let discovered_at = chrono::DateTime::parse_from_rfc3339(
                value["discovered_at"]
                    .as_str()
                    .context("planner seed without date")?,
            )?
            .with_timezone(&chrono::Utc);
            anyhow::ensure!(source.len() <= 256, "planner seed provenance exceeds limit");
            batch.push((url, value, discovered_at));
            if batch.len() == BATCH_SIZE {
                count += self.import_plan_batch(&batch)?;
                batch.clear();
            }
        }
        count += self.import_plan_batch(&batch)?;
        Ok(count)
    }
    fn import_plan_batch(
        &self,
        batch: &[(Url, serde_json::Value, chrono::DateTime<chrono::Utc>)],
    ) -> Result<u64> {
        self.transaction(|tx, stats| {
            let mut count = 0;
            for (url, provenance, date) in batch {
                let key = crate::capture::url_key(url)?;
                if read::<super::discovery::Candidate>(tx, CANDIDATES, &key)?.is_some() {
                    continue;
                }
                let candidate = super::discovery::Candidate {
                    url: url.clone(),
                    source: url.clone(),
                    discovered_at: *date,
                    lastmod: None,
                    sitemap: false,
                    depth: 0,
                };
                put(tx, CANDIDATES, &key, &candidate)?;
                put(tx, META, &format!("seed:{key}"), provenance)?;
                if self.admit(tx, stats, url, true)? {
                    let mut row: UrlRow =
                        read(tx, URLS, &key)?.context("imported planner URL missing")?;
                    row.weight = provenance["weight"]
                        .as_f64()
                        .filter(|n| n.is_finite() && *n >= 0.0)
                        .unwrap_or(1.0);
                    put(tx, URLS, &key, &row)?;
                }
                tx.open_table(CANDIDATE_READY)?
                    .insert((Domain::from(url).as_str(), key.as_str()), ())?;
                stats.discovery_candidates += 1;
                count += 1;
            }
            Ok(count)
        })
    }
    pub fn candidate_urls(
        &self,
        source: &Url,
        limit: usize,
    ) -> Result<Vec<super::discovery::Candidate>> {
        let tx = self.db.begin_read()?;
        let sources = tx.open_table(CANDIDATE_SOURCE)?;
        let candidates = tx.open_table(CANDIDATES)?;
        sources
            .range((source.as_str(), "")..=(source.as_str(), "\u{10ffff}"))?
            .take(limit.min(512))
            .map(|r| {
                let (key, _) = r?;
                let (_, url) = key.value();
                let value = candidates
                    .get(url)?
                    .context("discovery source without candidate")?;
                Ok(serde_json::from_slice(value.value())?)
            })
            .collect()
    }
    pub fn pending_live_candidates(
        &self,
        tenant: &Domain,
        now: u64,
        limit: usize,
    ) -> Result<Vec<super::discovery::Candidate>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(LIVE_PENDING)?;
        let candidates = tx.open_table(CANDIDATES)?;
        table
            .range((tenant.as_str(), "")..=(tenant.as_str(), "\u{10ffff}"))?
            .filter_map(|item| match item {
                Ok((key, due)) if due.value() <= now => Some((|| {
                    let value = candidates
                        .get(key.value().1)?
                        .context("missing durable live candidate")?;
                    Ok(serde_json::from_slice(value.value())?)
                })()),
                Ok(_) => None,
                Err(e) => Some(Err(e.into())),
            })
            .take(limit.min(512))
            .collect()
    }
    pub fn settle_live_candidate(&self, url: &Url, due: Option<u64>) -> Result<()> {
        let key = crate::capture::url_key(url)?;
        self.transaction(|tx, _| {
            let mut table = tx.open_table(LIVE_PENDING)?;
            let tenant = Domain::from(url);
            if let Some(due) = due {
                if table.get((tenant.as_str(), key.as_str()))?.is_some() {
                    table.insert((tenant.as_str(), key.as_str()), due)?;
                }
            } else {
                table.remove((tenant.as_str(), key.as_str()))?;
            }
            Ok(())
        })
    }
    pub fn process_controls(&self, directory: &Path) -> Result<()> {
        if !directory.exists() {
            return Ok(());
        }
        let mut processed = false;
        for entry in fs::read_dir(directory)?
            .filter(|entry| {
                entry.as_ref().map_or(true, |entry| {
                    entry
                        .path()
                        .extension()
                        .is_some_and(|extension| extension == "request")
                })
            })
            .take(128)
        {
            let path = entry?.path();
            if path
                .extension()
                .is_none_or(|extension| extension != "request")
            {
                continue;
            }
            let result = (|| -> Result<serde_json::Value> {
                anyhow::ensure!(
                    path.metadata()?.len() <= 16384,
                    "crawler control request exceeds limit"
                );
                let request: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
                let id = request["id"]
                    .as_str()
                    .context("control request without id")?;
                uuid::Uuid::parse_str(id)?;
                anyhow::ensure!(
                    path.file_stem().and_then(|v| v.to_str()) == Some(id),
                    "control filename/id mismatch"
                );
                let url = Url::parse(
                    request["url"]
                        .as_str()
                        .context("control request without URL")?,
                )?;
                anyhow::ensure!(
                    request["action"] == "release_quarantine",
                    "unsupported crawler control action"
                );
                let released = self.release_quarantine(&url)?;
                Ok(
                    serde_json::json!({"id":id,"url":url,"state":if released { "completed" } else { "not_found" }}),
                )
            })();
            let value = match result {
                Ok(v) => v,
                Err(error) => serde_json::json!({"state":"rejected", "error":error.to_string()}),
            };
            let mut file = fs::File::create(path.with_extension("result.tmp"))?;
            use std::io::Write;
            file.write_all(&serde_json::to_vec(&value)?)?;
            file.sync_all()?;
            fs::rename(
                path.with_extension("result.tmp"),
                path.with_extension("result"),
            )?;
            fs::remove_file(path)?;
            processed = true;
        }
        if processed {
            fs::File::open(directory)?.sync_all()?;
        }
        Ok(())
    }
    pub fn candidate_count(&self) -> Result<u64> {
        Ok(self.db.begin_read()?.open_table(CANDIDATES)?.len()?)
    }
    fn discovery_batch(
        &self,
        source: &Url,
        depth: u32,
        batch: &[crate::sitemap::SitemapEntry],
    ) -> Result<u64> {
        let config = self.discovery_config.lock().unwrap().clone();
        let origin = source.origin().ascii_serialization();
        self.transaction(|tx, stats| {
            let mut cycle: super::discovery::Cycle =
                read(tx, DISCOVERY_CYCLES, &origin)?.context("missing discovery cycle")?;
            let mut inserted = 0;
            for entry in batch {
                let (url, lastmod, sitemap) = match entry {
                    crate::sitemap::SitemapEntry::Url(dated) => {
                        (&dated.url, dated.last_modified, false)
                    }
                    crate::sitemap::SitemapEntry::Sitemap(url) => (url, None, true),
                };
                let key = crate::capture::url_key(url)?;
                if let Some(mut previous) =
                    read::<super::discovery::Candidate>(tx, CANDIDATES, &key)?
                {
                    tx.open_table(CANDIDATE_SOURCE)?
                        .insert((source.as_str(), key.as_str()), ())?;
                    if lastmod > previous.lastmod {
                        previous.lastmod = lastmod;
                        put(tx, CANDIDATES, &key, &previous)?;
                    }
                    continue;
                }
                if cycle.urls >= config.max_urls || (sitemap && depth + 1 >= config.max_depth) {
                    stats.discovery_quota_stops += 1;
                    continue;
                }
                let candidate = super::discovery::Candidate {
                    url: url.clone(),
                    source: source.clone(),
                    discovered_at: chrono::Utc::now(),
                    lastmod,
                    sitemap,
                    depth: depth + 1,
                };
                put(tx, CANDIDATES, &key, &candidate)?;
                if !sitemap {
                    tx.open_table(LIVE_PENDING)?
                        .insert((Domain::from(url).as_str(), key.as_str()), 0)?;
                }
                tx.open_table(CANDIDATE_SOURCE)?
                    .insert((source.as_str(), key.as_str()), ())?;
                tx.open_table(CANDIDATE_READY)?
                    .insert((Domain::from(url).as_str(), key.as_str()), ())?;
                cycle.urls += 1;
                stats.discovery_candidates += 1;
                inserted += 1;
                if self.admit(tx, stats, url, true)? && sitemap {
                    let mut row: UrlRow =
                        read(tx, URLS, &key)?.context("missing sitemap candidate")?;
                    row.sitemap = true;
                    row.sitemap_depth = depth + 1;
                    put(tx, URLS, &key, &row)?;
                }
                if read::<UrlRow>(tx, URLS, &key)?.is_some_and(|r| r.round == stats.round) {
                    tx.open_table(CANDIDATE_READY)?
                        .remove((Domain::from(url).as_str(), key.as_str()))?;
                }
            }
            put(tx, DISCOVERY_CYCLES, &origin, &cycle)?;
            Ok(inserted)
        })
    }
    pub fn ingest_sitemap(&self, source: &Url, text: &str) -> Result<super::discovery::Report> {
        self.ingest_discovery(source, text, None)
    }
    pub fn ingest_feed(
        &self,
        source: &Url,
        text: &str,
        kind: crate::feed::FeedKind,
    ) -> Result<super::discovery::Report> {
        self.ingest_discovery(source, text, Some(kind))
    }
    fn ingest_discovery(
        &self,
        source: &Url,
        text: &str,
        kind: Option<crate::feed::FeedKind>,
    ) -> Result<super::discovery::Report> {
        let config = self.discovery_config.lock().unwrap().clone();
        let origin = source.origin().ascii_serialization();
        let key = crate::capture::url_key(source)?;
        let document_key = format!("{key}:{}", crate::capture::sha256(text.as_bytes()));
        let cycle_id = (chrono::Utc::now().timestamp_millis().max(0) as u64) / config.cycle_ms;
        let admission = self.transaction(|tx, stats| {
            let mut cycle: super::discovery::Cycle =
                read(tx, DISCOVERY_CYCLES, &origin)?.unwrap_or_default();
            if cycle.round != cycle_id {
                cycle = super::discovery::Cycle {
                    round: cycle_id,
                    ..Default::default()
                };
            }
            if tx
                .open_table(DOCUMENTS)?
                .get(document_key.as_str())?
                .is_some_and(|v| v.value() == cycle_id)
            {
                return Ok(Err("already_processed"));
            }
            let depth = read::<UrlRow>(tx, URLS, &key)?.map_or(0, |r| r.sitemap_depth);
            if depth >= config.max_depth
                || cycle.documents >= config.max_documents
                || cycle.bytes.saturating_add(text.len() as u64) > config.max_total_bytes
            {
                stats.discovery_quota_stops += 1;
                return Ok(Err("discovery_aggregate_quota"));
            }
            cycle.documents += 1;
            cycle.bytes += text.len() as u64;
            stats.discovery_documents += 1;
            stats.discovery_bytes += text.len() as u64;
            put(tx, DISCOVERY_CYCLES, &origin, &cycle)?;
            tx.open_table(DOCUMENTS)?
                .insert(document_key.as_str(), cycle_id)?;
            Ok(Ok(depth))
        })?;
        let before_stops = self.metrics.snapshot().discovery_quota_stops;
        let depth = match admission {
            Ok(depth) => depth,
            Err(reason) => {
                return Ok(super::discovery::Report {
                    candidates: 0,
                    reason: reason.into(),
                })
            }
        };
        let mut batch = Vec::with_capacity(BATCH_SIZE);
        let mut candidates = 0;
        let mut emit = |entry| -> Result<()> {
            batch.push(entry);
            if batch.len() >= BATCH_SIZE {
                candidates += self
                    .discovery_batch(source, depth, &batch)
                    .map_err(super::discovery::StorageError)?;
                batch.clear();
            }
            Ok(())
        };
        let result = match kind {
            Some(kind) => crate::feed::visit(text, kind, |entry| {
                emit(crate::sitemap::SitemapEntry::Url(entry))
            }),
            None => crate::sitemap::visit_sitemap(text.as_bytes(), emit),
        };
        candidates += self.discovery_batch(source, depth, &batch)?;
        let reason = match result {
            Ok(_) if self.metrics.snapshot().discovery_quota_stops > before_stops => {
                "discovery_aggregate_quota"
            }
            Ok(_) => "complete",
            Err(error)
                if error
                    .downcast_ref::<super::discovery::StorageError>()
                    .is_some() =>
            {
                return Err(error)
            }
            Err(_) => {
                self.transaction(|_, stats| {
                    stats.discovery_invalid_xml += 1;
                    Ok(())
                })?;
                "invalid_discovery_xml"
            }
        };
        Ok(super::discovery::Report {
            candidates,
            reason: reason.into(),
        })
    }

    pub fn configure_recrawl(&self, config: &super::recrawl::Config) -> Result<()> {
        config.validate()?;
        *self.recrawl_config.lock().unwrap() = config.clone();
        Ok(())
    }
    pub fn revalidation_state(&self, url: &Url) -> Result<Option<super::recrawl::Resource>> {
        let key = crate::capture::url_key(url)?;
        self.db
            .begin_read()?
            .open_table(RECRAWL)?
            .get(key.as_str())?
            .map(|v| serde_json::from_slice(v.value()).map_err(Into::into))
            .transpose()
    }
    fn record_resource(
        &self,
        tx: &WriteTransaction,
        stats: &mut Stats,
        metadata: &crate::capture::Metadata,
        receipt: &Receipt,
    ) -> Result<()> {
        let config = self.recrawl_config.lock().unwrap().clone();
        let key = metadata.requested_url.as_str();
        let previous: Option<super::recrawl::Resource> = read(tx, RECRAWL, key)?;
        let mime = metadata
            .response
            .values("content-type")
            .next()
            .or_else(|| previous.as_ref().and_then(|r| r.mime.clone()));
        let value = read::<UrlRow>(tx, URLS, key)?.map_or(1.0, |row| row.weight);
        let bytes = metadata
            .revalidation
            .as_ref()
            .map_or(metadata.payload_bytes, |reference| reference.payload_bytes);
        let (initial_interval, max_interval) =
            config.bounds(value, mime.as_deref(), metadata.fetch_time_ms, bytes);
        let now = millis(metadata.response.received_at);
        let mut resource = if let Some(mut old) = previous {
            tx.open_table(RECRAWL_READY)?
                .remove((old.next_eligible_ms, key))?;
            let changed = metadata.revalidation.is_none()
                && old.reference.payload_sha256 != metadata.payload_sha256;
            old.checks += 1;
            if changed {
                old.changes += 1;
                stats.recrawl_changes += 1;
            }
            old.interval_ms = if changed {
                (old.interval_ms / 2)
                    .min(initial_interval)
                    .max(config.min_interval_ms)
            } else {
                old.interval_ms
                    .saturating_mul(3)
                    .saturating_div(2)
                    .min(max_interval)
            };
            old
        } else {
            super::recrawl::Resource {
                mime: mime.clone(),
                reference: super::recrawl::PayloadReference {
                    version: 1,
                    fetch_id: metadata.fetch_id.clone(),
                    segment: receipt.segment.clone(),
                    record_offset: receipt.record_offset.unwrap_or(receipt.start),
                    record_end: receipt.end,
                    payload_sha256: metadata.payload_sha256.clone(),
                    payload_bytes: metadata.payload_bytes,
                    url: metadata.response.url.clone(),
                },
                etag: None,
                last_modified: None,
                robots: metadata.robots.clone(),
                checks: 1,
                changes: 0,
                last_checked_ms: now,
                next_eligible_ms: now,
                interval_ms: initial_interval,
            }
        };
        resource.mime = mime;
        if let Some(reference) = &metadata.revalidation {
            anyhow::ensure!(
                reference.fetch_id == resource.reference.fetch_id
                    && reference.payload_sha256 == resource.reference.payload_sha256,
                "304 reference conflicts with durable resource"
            );
            stats.revalidations += 1;
            stats.recrawl_bytes_saved += reference.payload_bytes as u64;
        } else {
            resource.reference = super::recrawl::PayloadReference {
                version: 1,
                fetch_id: metadata.fetch_id.clone(),
                segment: receipt.segment.clone(),
                record_offset: receipt.record_offset.unwrap_or(receipt.start),
                record_end: receipt.end,
                payload_sha256: metadata.payload_sha256.clone(),
                payload_bytes: metadata.payload_bytes,
                url: metadata.response.url.clone(),
            };
            let version = crate::capture::sha256(&serde_json::to_vec(&(
                metadata.schema_version,
                &metadata.requested_url,
                &metadata.response.headers,
                &metadata.robots,
                &metadata.payload_sha256,
            ))?);
            tx.open_table(METADATA_VERSIONS)?
                .insert(version.as_str(), ())?;
            let payload = metadata.payload_sha256.as_str();
            let duplicate = tx.open_table(CONTENT)?.get(payload)?.is_some();
            if duplicate {
                stats.duplicate_content += 1;
            } else {
                tx.open_table(CONTENT)?
                    .insert(payload, metadata.fetch_id.as_str())?;
                stats.content_versions += 1;
            }
            let (pattern, _) = self
                .traps_config
                .lock()
                .unwrap()
                .pattern(&url::Url::parse(&metadata.response.url)?);
            let mut family: super::traps::Family =
                read(tx, FAMILIES, &pattern)?.unwrap_or_default();
            if duplicate {
                family.repeated += 1;
            }
            if metadata.decoded_bytes == 0 {
                family.empty += 1;
                stats.empty_pages += 1;
            }
            put(tx, FAMILIES, &pattern, &family)?;
        }
        let etag = metadata.response.values("etag").next();
        let modified = metadata.response.values("last-modified").next();
        if metadata.revalidation.is_none() || etag.is_some() {
            resource.etag = etag;
        }
        if metadata.revalidation.is_none() || modified.is_some() {
            resource.last_modified = modified;
        }
        resource.robots = metadata.robots.clone();
        resource.last_checked_ms = now;
        resource.next_eligible_ms = now.saturating_add(resource.interval_ms);
        if config.enabled {
            tx.open_table(RECRAWL_READY)?
                .insert((resource.next_eligible_ms, key), ())?;
        }
        put(tx, RECRAWL, key, &resource)
    }
    pub fn requeue_recrawls(&self, now: chrono::DateTime<chrono::Utc>) -> Result<usize> {
        if !self.recrawl_config.lock().unwrap().enabled {
            return Ok(0);
        }
        self.transaction(|tx, stats| {
            let batch = tx
                .open_table(RECRAWL_READY)?
                .range(..=(millis(now), "\u{10ffff}"))?
                .take(BATCH_SIZE)
                .map(|r| r.map(|(k, _)| (k.value().0, k.value().1.to_string())))
                .collect::<std::result::Result<Vec<_>, _>>()?;
            let mut count = 0;
            for (due, key) in batch {
                let Some(mut row): Option<UrlRow> = read(tx, URLS, &key)? else {
                    continue;
                };
                if row.state != State::Committed {
                    tx.open_table(RECRAWL_READY)?.remove((due, key.as_str()))?;
                    continue;
                }
                let mut origin: OriginRow =
                    read(tx, ORIGINS, &row.site)?.context("recrawl origin missing")?;
                if origin.state == "active" {
                    continue;
                }
                if origin.round != stats.round {
                    origin.round = stats.round;
                    origin.admitted = 0;
                    origin.done = 0;
                    stats.known_sites += 1;
                }
                if origin.state == "done" {
                    if stats.pending_sites + stats.active_sites >= self.capacity as u64 {
                        continue;
                    }
                    origin.state = "pending".into();
                    stats.pending_sites += 1;
                } else {
                    tx.open_table(READY)?
                        .remove((origin.not_before, origin.sequence))?;
                }
                stats.sequence += 1;
                origin.sequence = stats.sequence;
                origin.not_before = origin.not_before.min(due);
                tx.open_table(READY)?
                    .insert((origin.not_before, origin.sequence), row.site.as_str())?;
                put(tx, ORIGINS, &row.site, &origin)?;
                row.round = stats.round;
                row.state = State::Pending;
                row.fetch_id = None;
                row.retry = Default::default();
                row.reason = Some("recrawl_due".into());
                row.not_before = due;
                stats.committed_urls -= 1;
                stats.pending_urls += 1;
                tx.open_table(ELIGIBLE)?
                    .insert((row.site.as_str(), key.as_str()), due)?;
                Self::queue_http_origin(tx, stats, &row)?;
                put(tx, URLS, &key, &row)?;
                tx.open_table(RECRAWL_READY)?.remove((due, key.as_str()))?;
                count += 1;
            }
            Ok(count)
        })
    }

    pub fn configure_retry(&self, config: &super::retry::Config) -> Result<()> {
        config.validate()?;
        *self.retry_config.lock().unwrap() = config.clone();
        Ok(())
    }

    pub fn authorize_request(
        &self,
        requested: &Url,
        destination: &Url,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<super::Result<()>> {
        let config = self.retry_config.lock().unwrap().clone();
        let key = crate::capture::url_key(requested)?;
        let origin = destination.origin().ascii_serialization();
        let now = millis(now);
        self.transaction(|tx, stats| {
            let mut row: UrlRow =
                read(tx, URLS, &key)?.context("request without durable URL admission")?;
            let mut state: super::retry::OriginState =
                read(tx, RETRIES, &origin)?.unwrap_or_default();
            if state.not_before_ms > now {
                return Ok(Err(super::Error::CrawlDeferred {
                    retry_after_ms: state.not_before_ms - now,
                }));
            }
            if row.retry.failures >= config.max_attempts
                || (row.retry.first_attempt_ms > 0
                    && now.saturating_sub(row.retry.first_attempt_ms) >= config.max_elapsed_ms)
            {
                return Ok(Err(super::Error::RetryExhausted(key.clone())));
            }
            if now.saturating_sub(state.window_start_ms) >= 60_000 {
                state.window_start_ms = now;
                state.window_attempts = 0;
            }
            let mut global: super::retry::OriginState =
                read(tx, META, "request-budget.v1")?.unwrap_or_default();
            if now.saturating_sub(global.window_start_ms) >= 60_000 {
                global.window_start_ms = now;
                global.window_attempts = 0;
            }
            if state.window_attempts >= config.origin_attempts_per_minute {
                return Ok(Err(super::Error::CrawlDeferred {
                    retry_after_ms: state
                        .window_start_ms
                        .saturating_add(60_000)
                        .saturating_sub(now),
                }));
            }
            if global.window_attempts >= config.global_attempts_per_minute {
                return Ok(Err(super::Error::CrawlDeferred {
                    retry_after_ms: global
                        .window_start_ms
                        .saturating_add(60_000)
                        .saturating_sub(now),
                }));
            }
            state.window_attempts += 1;
            global.window_attempts += 1;
            row.retry.attempts += 1;
            if row.retry.first_attempt_ms == 0 {
                row.retry.first_attempt_ms = now;
            }
            stats.http_attempts += 1;
            put(tx, URLS, &key, &row)?;
            put(tx, RETRIES, &origin, &state)?;
            put(tx, META, "request-budget.v1", &global)?;
            Ok(Ok(()))
        })
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
        if old.as_ref().is_some_and(|row| {
            row.round == stats.round
                || matches!(
                    row.reason.as_deref(),
                    Some("http_410" | "PermanentFailure { reason: \"http_410\" }")
                )
        }) {
            return Ok(false);
        }
        if old.is_none() && !self.trap_admission(tx, stats, url)? {
            return Ok(false);
        }
        let due = read::<super::recrawl::Resource>(tx, RECRAWL, &key)?
            .map_or(0, |r| r.next_eligible_ms)
            .max(old.as_ref().map_or(0, |r| r.not_before));
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
                    .insert((due, stats.sequence), site.as_str())?;
                stats.known_sites += 1;
                stats.pending_sites += 1;
                OriginRow {
                    sequence: stats.sequence,
                    round: stats.round,
                    state: "pending".into(),
                    seed: key.clone(),
                    admitted: 0,
                    done: 0,
                    not_before: due,
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
                weight: 1.0,
                url: key.clone(),
                origin: url.origin().ascii_serialization(),
                site: site.clone(),
                round: stats.round,
                state: State::Pending,
                fetch_id: None,
                reason: None,
                not_before: due,
                sitemap: false,
                retry: Default::default(),
                sitemap_depth: 0,
                continuation: None,
            },
        )?;
        tx.open_table(ELIGIBLE)?
            .insert((site.as_str(), key.as_str()), due)?;
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
                let key = crate::capture::url_key(&Url::parse(&metadata.requested_url)?)?;
                if read::<super::recrawl::Resource>(tx, RECRAWL, &key)?.is_none()
                    && read::<UrlRow>(tx, URLS, &key)?
                        .is_some_and(|r| r.fetch_id.as_deref() == Some(metadata.fetch_id.as_str()))
                {
                    self.record_resource(tx, stats, metadata, receipt)?;
                }
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
            self.record_resource(tx, stats, metadata, receipt)?;
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
                            weight: row.weight,
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
        let retry_config = self.retry_config.lock().unwrap().clone();
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
                if matches!(entry.outcome, FetchOutcome::DurableSuccess { .. }) {
                    let retry_origin = match &entry.outcome {
                        FetchOutcome::DurableSuccess { final_url } => {
                            final_url.origin().ascii_serialization()
                        }
                        _ => row.origin.clone(),
                    };
                    let mut retry: super::retry::OriginState =
                        read(tx, RETRIES, &retry_origin)?.unwrap_or_default();
                    if retry.consecutive_failures > 0 {
                        stats.retry_recovered += 1;
                    }
                    retry.success(millis(now));
                    delay = delay.max(retry.penalty_ms);
                    put(tx, RETRIES, &retry_origin, &retry)?;
                }
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
                        if super::retry::is_transient(reason) {
                            row.retry.failures = row.retry.failures.saturating_add(1);
                            if row.retry.first_attempt_ms == 0 {
                                row.retry.first_attempt_ms = millis(now);
                            }
                            delay = delay.max(retry_config.delay(
                                row.retry.failures,
                                retry_after_ms.unwrap_or(0),
                                &row.url,
                            ));
                            let retry_origin =
                                row.continuation.as_ref().map_or(row.origin.clone(), |c| {
                                    c.url.origin().ascii_serialization()
                                });
                            let mut retry: super::retry::OriginState =
                                read(tx, RETRIES, &retry_origin)?.unwrap_or_default();
                            if retry.failure(&retry_config, millis(now), delay) {
                                stats.circuits_opened += 1;
                            }
                            delay = delay.max(retry.not_before_ms.saturating_sub(millis(now)));
                            put(tx, RETRIES, &retry_origin, &retry)?;
                        }
                        if row.retry.failures >= retry_config.max_attempts
                            || (row.retry.first_attempt_ms > 0
                                && millis(now).saturating_sub(row.retry.first_attempt_ms)
                                    >= retry_config.max_elapsed_ms)
                        {
                            row.state = State::Terminal;
                            row.reason = Some(format!("retry_budget_exhausted:{reason}"));
                            row.continuation = None;
                            stats.retry_exhausted += 1;
                            stats.terminal_urls += 1;
                            completed += 1;
                        } else {
                            row.state = State::Pending;
                            row.reason = Some(reason.clone());
                            stats.pending_urls += 1;
                            stats.retry_deferred += 1;
                        }
                    }
                    FetchOutcome::Cancelled => {
                        row.state = State::Pending;
                        stats.pending_urls += 1;
                    }
                    other => {
                        row.state = State::Terminal;
                        row.continuation = None;
                        row.reason = Some(match other { FetchOutcome::PermanentFailure { reason } | FetchOutcome::Skipped { reason } => reason.clone(), _ => format!("{other:?}") });
                        if matches!(other, FetchOutcome::PermanentFailure { reason } if reason == "http_404") {
                            row.not_before = millis(now).saturating_add(604_800_000);
                        }
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
            if !remaining {
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
                    if receipt.capture_version.is_none() {
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
