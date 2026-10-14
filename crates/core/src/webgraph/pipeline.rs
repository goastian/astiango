//! Exact capture ledger -> immutable compatible graph generations. Source ACKs
//! belong to the ledger; Tantivy is a rebuildable projection with its own readiness.
use super::{metrics, Edge, Node, Webgraph};
use crate::{
    capture,
    config::{LocalConfig, WarcSource},
    warc::{
        consumer::{self, Consumer},
        stream::Limits,
        WarcRecord,
    },
    webpage::{html::links::RelFlags, Html},
    Result,
};
use anyhow::Context;
use fs4::FileExt;
use redb::{Database, ReadableTable, ReadableTableMetadata, TableDefinition, WriteTransaction};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Seek, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};
use url::Url;

pub static CANCELLED: AtomicBool = AtomicBool::new(false);
pub mod fixture;
const DOCS: TableDefinition<&str, &[u8]> = TableDefinition::new("graph.versions.v1");
const CURRENT: TableDefinition<&str, &[u8]> = TableDefinition::new("graph.current.v1");
const CAPTURES: TableDefinition<&str, &str> = TableDefinition::new("graph.captures.v1");
const RECORDS: TableDefinition<(&str, u64), &[u8]> = TableDefinition::new("graph.records.v1");
const EDGES: TableDefinition<(&str, &str), &[u8]> = TableDefinition::new("graph.edges.v1");
const ACTIVE: TableDefinition<(u64, &str, &str), &[u8]> =
    TableDefinition::new("graph.active-edges.v1");
const HOSTS: TableDefinition<(&str, &str), ()> = TableDefinition::new("graph.hosts.v1");
const COUNTS: TableDefinition<&str, u64> = TableDefinition::new("graph.counts.v1");
const SOURCES: TableDefinition<&str, &[u8]> = TableDefinition::new("graph.sources.v1");
const DIRTY: TableDefinition<u64, ()> = TableDefinition::new("graph.dirty.v1");
const META: TableDefinition<&str, &str> = TableDefinition::new("graph.meta.v1");

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub output_path: PathBuf,
    pub source_directory: PathBuf,
    pub files: Vec<String>,
    pub consumer: Limits,
    pub writer_bytes: usize,
    pub cache_bytes: usize,
    pub memory_budget_bytes: usize,
    pub max_links_per_document: usize,
    pub edge_bytes_per_document: usize,
    pub buckets: u64,
    pub max_output_bytes: u64,
    pub disk_reserve_bytes: u64,
    pub temporary_bytes: u64,
    pub report_path: Option<PathBuf>,
    pub publish_generations: bool,
    pub max_centrality_edges: u64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            output_path: PathBuf::new(),
            source_directory: PathBuf::new(),
            files: vec![],
            consumer: Limits {
                workers: 1,
                decoded_bytes: 4 * 1024 * 1024,
                batch_bytes: 16 * 1024 * 1024,
                ..Limits::default()
            },
            writer_bytes: 24 * 1024 * 1024,
            cache_bytes: 4 * 1024 * 1024,
            memory_budget_bytes: 512 * 1024 * 1024,
            max_links_per_document: 200_000,
            edge_bytes_per_document: 64 * 1024 * 1024,
            buckets: 64,
            max_output_bytes: 4 * 1024 * 1024 * 1024,
            disk_reserve_bytes: 1024 * 1024 * 1024,
            temporary_bytes: 1024 * 1024 * 1024,
            report_path: None,
            publish_generations: false,
            max_centrality_edges: 2000,
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        self.consumer.validate()?;
        anyhow::ensure!(
            self.consumer.workers == 1
                && (15 * 1024 * 1024..=64 * 1024 * 1024).contains(&self.writer_bytes),
            "graph requires one bounded writer (15..64 MiB)"
        );
        anyhow::ensure!(
            (1024 * 1024..=64 * 1024 * 1024).contains(&self.cache_bytes)
                && (1..=128).contains(&self.buckets)
                && self.max_links_per_document > 0
                && self.edge_bytes_per_document > 0,
            "invalid graph cache/partition/link budget"
        );
        let reserve = self
            .consumer
            .decoded_bytes
            .saturating_mul(64)
            .saturating_add(self.consumer.record_bytes)
            .saturating_add(self.writer_bytes * 2)
            .saturating_add(self.cache_bytes)
            .saturating_add(16 * 1024 * 1024);
        anyhow::ensure!(
            reserve <= self.memory_budget_bytes,
            "graph decoded/DOM/writers/cache exceed combined memory budget"
        );
        anyhow::ensure!(
            self.max_output_bytes > 0
                && self.temporary_bytes >= self.writer_bytes as u64 * 2
                && !self.output_path.as_os_str().is_empty()
                && self.source_directory.is_dir(),
            "invalid graph paths or disk/merge budget"
        );
        anyhow::ensure!(
            self.files.len() <= 128
                && self
                    .files
                    .iter()
                    .all(|v| Path::new(v).file_name().and_then(|v| v.to_str()) == Some(v.as_str())),
            "graph inputs must be 1..128 local filenames"
        );
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub url: String,
    pub final_url: String,
    pub fetch_id: String,
    pub captured_at: i64,
    pub payload_sha256: String,
    pub source: String,
    pub ordinal: u64,
    pub bucket: u64,
    pub edges: u64,
    pub host_degree: u64,
    pub state: String,
    pub canonical: Option<String>,
    pub metadata: Option<capture::Metadata>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Current {
    pub version: String,
    pub observed_at: i64,
    pub fetch_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Decision {
    pub source: String,
    pub ordinal: u64,
    pub fetch_id: String,
    pub url: String,
    pub version: Option<String>,
    pub outcome: String,
    pub reason: Option<String>,
    pub auxiliary: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct StoredEdge {
    pub id: String,
    pub source_url: String,
    pub destination_url: String,
    pub from: Node,
    pub to: Node,
    pub rel: RelFlags,
    pub label: String,
}
impl StoredEdge {
    fn edge(&self, degree: u64) -> Edge {
        Edge {
            from: self.from.clone(),
            to: self.to.clone(),
            rel_flags: self.rel,
            label: self.label.clone(),
            sort_score: u64::MAX,
            from_centrality: 0.0,
            to_centrality: 0.0,
            from_rank: u64::MAX,
            to_rank: u64::MAX,
            num_outgoing_hosts_from_page: degree,
        }
    }
}
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Source {
    pub name: String,
    pub fingerprint: String,
    pub records: u64,
    pub complete: bool,
    pub error: Option<String>,
}
#[derive(Serialize)]
pub struct Report {
    pub version: u32,
    pub revision: u64,
    pub stage: String,
    pub documents: u64,
    pub current_edges: u64,
    pub source_records: u64,
    pub source_complete: bool,
    pub outcomes: BTreeMap<String, u64>,
    pub elapsed_ms: f64,
    pub cpu_ms: f64,
    pub allocated_bytes: u64,
    pub stages: BTreeMap<String, metrics::Measurement>,
    pub error: Option<String>,
}
struct Ledger {
    db: Database,
}
fn get<T: serde::de::DeserializeOwned>(
    tx: &WriteTransaction,
    table: TableDefinition<&str, &[u8]>,
    key: &str,
) -> Result<Option<T>> {
    let t = tx.open_table(table)?;
    let result = t
        .get(key)?
        .map(|v| serde_json::from_slice(v.value()).map_err(Into::into))
        .transpose();
    result
}
fn put<T: Serialize>(
    tx: &WriteTransaction,
    table: TableDefinition<&str, &[u8]>,
    key: &str,
    value: &T,
) -> Result<()> {
    let bytes = serde_json::to_vec(value)?;
    tx.open_table(table)?.insert(key, bytes.as_slice())?;
    Ok(())
}
impl Ledger {
    fn open(c: &Config) -> Result<Self> {
        let db = Database::builder()
            .set_cache_size(c.cache_bytes)
            .create(c.output_path.join("ledger.redb"))?;
        let tx = db.begin_write()?;
        {
            let mut meta = tx.open_table(META)?;
            if let Some(v) = meta.get("version")? {
                anyhow::ensure!(v.value() == "1", "unknown graph ledger version");
            }
            meta.insert("version", "1")?;
            let previous = meta.get("buckets")?.map(|v| v.value().to_owned());
            let buckets = c.buckets.to_string();
            anyhow::ensure!(
                previous.as_deref().is_none_or(|v| v == buckets),
                "partition count changed; generation migration required"
            );
            meta.insert("buckets", buckets.as_str())?;
        }
        tx.open_table(DOCS)?;
        tx.open_table(CURRENT)?;
        tx.open_table(CAPTURES)?;
        tx.open_table(RECORDS)?;
        tx.open_table(EDGES)?;
        tx.open_table(ACTIVE)?;
        tx.open_table(HOSTS)?;
        tx.open_table(COUNTS)?;
        tx.open_table(SOURCES)?;
        tx.open_table(DIRTY)?;
        tx.commit()?;
        Ok(Self { db })
    }
    fn report(
        &self,
        stage: &str,
        source: &Source,
        start: Instant,
        cpu: u64,
        error: Option<String>,
    ) -> Result<Report> {
        let tx = self.db.begin_read()?;
        let counts = tx
            .open_table(COUNTS)?
            .iter()?
            .map(|v| v.map(|(k, v)| (k.value().to_owned(), v.value())))
            .collect::<std::result::Result<BTreeMap<_, _>, _>>()?;
        let revision = tx
            .open_table(META)?
            .get("revision")?
            .map(|v| v.value().parse::<u64>())
            .transpose()?
            .unwrap_or(0);
        Ok(Report {
            version: 1,
            revision,
            stage: stage.into(),
            documents: tx.open_table(CURRENT)?.len()?,
            current_edges: tx.open_table(ACTIVE)?.len()?,
            source_records: source.records,
            source_complete: source.complete,
            outcomes: counts,
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
            cpu_ms: metrics::cpu_ns().saturating_sub(cpu) as f64 / 1e6,
            allocated_bytes: metrics::ALLOCATED.load(Ordering::Relaxed),
            stages: metrics::snapshot(),
            error,
        })
    }
}
fn increment(tx: &WriteTransaction, key: &str) -> Result<()> {
    let mut t = tx.open_table(COUNTS)?;
    let n = t.get(key)?.map_or(0, |v| v.value());
    t.insert(key, n + 1)?;
    Ok(())
}
fn decide(tx: &WriteTransaction, d: &Decision) -> Result<()> {
    let mut t = tx.open_table(RECORDS)?;
    if t.get((d.source.as_str(), d.ordinal))?.is_none() {
        let bytes = serde_json::to_vec(d)?;
        t.insert((d.source.as_str(), d.ordinal), bytes.as_slice())?;
        drop(t);
        increment(
            tx,
            if d.auxiliary {
                "reference_restored"
            } else {
                &d.outcome
            },
        )?;
    }
    Ok(())
}
pub fn tree_bytes(root: &Path) -> Result<u64> {
    use std::os::unix::fs::MetadataExt;
    let mut stack = vec![root.to_path_buf()];
    let mut seen = HashSet::new();
    let mut total = 0u64;
    while let Some(p) = stack.pop() {
        for f in fs::read_dir(p)? {
            let f = f?;
            let kind = f.file_type()?;
            if kind.is_dir() {
                stack.push(f.path());
            } else if kind.is_file() {
                let m = f.metadata()?;
                if seen.insert((m.dev(), m.ino())) {
                    total = total.saturating_add(m.len());
                }
                anyhow::ensure!(seen.len() <= 100_000, "graph file count quota reached");
            }
        }
    }
    Ok(total)
}
fn capacity(c: &Config, reserve: u64) -> Result<()> {
    anyhow::ensure!(
        tree_bytes(&c.output_path)?.saturating_add(reserve) < c.max_output_bytes,
        "deferred: graph output/temporary reserve reached; confirmed data retained"
    );
    let path = std::ffi::CString::new(c.output_path.as_os_str().as_encoded_bytes())?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    anyhow::ensure!(
        unsafe { libc::statvfs(path.as_ptr(), &mut stat) } == 0,
        "local graph disk probe failed"
    );
    anyhow::ensure!(
        stat.f_bavail as u64 * stat.f_frsize as u64 >= c.disk_reserve_bytes.saturating_add(reserve),
        "deferred: local graph disk reserve reached"
    );
    Ok(())
}
fn release_unused_allocations() {
    // Independent bounded batches should not retain freed DOM/writer arenas.
    // This changes allocator residency, not the admitted body/link quotas.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    unsafe {
        libc::malloc_trim(0);
    }
}
fn check_control() -> Result<()> {
    anyhow::ensure!(
        !CANCELLED.load(Ordering::Relaxed),
        "cancelled: graph admission drained; confirmed data retained"
    );
    Ok(())
}
fn failpoint(name: &str) {
    if std::env::var("GRAPH_FAILPOINT").ok().as_deref() == Some(name) {
        unsafe {
            libc::kill(libc::getpid(), libc::SIGKILL);
        }
    }
}
fn project(c: &Config, value: &Report) {
    if let Some(p) = &c.report_path {
        let result = (|| -> Result<()> {
            let tmp = p.with_extension(format!("{}.tmp", std::process::id()));
            let mut output = BufWriter::with_capacity(65536, File::create(&tmp)?);
            serde_json::to_writer(&mut output, value)?;
            output.flush()?;
            drop(output);
            fs::rename(tmp, p)?;
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(%error,"graph metrics unavailable");
        }
    }
}

fn process_record(
    ledger: &Ledger,
    c: &Config,
    record: &WarcRecord,
    source: &str,
    ordinal: u64,
) -> Result<()> {
    check_control()?;
    if let Ok(delay) = std::env::var("GRAPH_TEST_DELAY_MS") {
        let delay: u64 = delay.parse()?;
        anyhow::ensure!(delay <= 2000, "invalid controlled test delay");
        for _ in 0..delay.div_ceil(10) {
            check_control()?;
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
    let metadata = record.metadata.capture.as_ref();
    let requested = metadata.map_or(record.request.url.as_str(), |m| m.requested_url.as_str());
    let url = capture::url_key(&Url::parse(requested)?)?;
    let final_url = capture::url_key(&Url::parse(&record.request.url)?)?;
    let at = metadata
        .map(|m| m.response.received_at.timestamp_millis())
        .or_else(|| record.request.date.map(|v| v.timestamp_millis()))
        .unwrap_or(0);
    let payload = metadata
        .map(|m| m.payload_sha256.clone())
        .unwrap_or_else(|| capture::sha256(record.response.body.as_bytes()));
    let fetch_id = metadata
        .map(|m| m.fetch_id.clone())
        .unwrap_or_else(|| capture::sha256(format!("{url}:{payload}:{at}").as_bytes()));
    let mut decision = Decision {
        source: source.into(),
        ordinal,
        fetch_id: fetch_id.clone(),
        url: url.clone(),
        version: None,
        outcome: "indexed".into(),
        reason: None,
        auxiliary: ordinal == 0,
    };
    {
        let read = ledger.db.begin_read()?;
        if read.open_table(RECORDS)?.get((source, ordinal))?.is_some() {
            return Ok(());
        }
        if read.open_table(CAPTURES)?.get(fetch_id.as_str())?.is_some() {
            decision.outcome = "duplicate_capture".into();
            let tx = ledger.db.begin_write()?;
            decide(&tx, &decision)?;
            tx.commit()?;
            return Ok(());
        }
    }
    let previous: Option<Current> = {
        let tx = ledger.db.begin_write()?;
        get(&tx, CURRENT, &url)?
    };
    if previous.as_ref().is_some_and(|v| v.observed_at > at) {
        decision.outcome = "obsolete".into();
        let tx = ledger.db.begin_write()?;
        decide(&tx, &decision)?;
        tx.commit()?;
        return Ok(());
    }
    if record.is_revalidation() {
        let reference = metadata
            .and_then(|m| m.revalidation.as_ref())
            .context("deferred: 304 without payload reference")?;
        let known = {
            let tx = ledger.db.begin_read()?;
            tx.open_table(CAPTURES)?
                .get(reference.fetch_id.as_str())?
                .is_some()
        };
        if !known {
            anyhow::ensure!(
                reference.version == 1
                    && reference.record_end > reference.record_offset
                    && reference.record_end - reference.record_offset
                        <= c.consumer.record_bytes as u64 * 3,
                "invalid graph 304 reference bounds/version"
            );
            anyhow::ensure!(
                Path::new(&reference.segment)
                    .file_name()
                    .and_then(|v| v.to_str())
                    == Some(reference.segment.as_str()),
                "invalid graph 304 reference path"
            );
            let path = c.source_directory.join(&reference.segment);
            anyhow::ensure!(
                path.canonicalize()?
                    .starts_with(c.source_directory.canonicalize()?),
                "graph reference escapes retained archive"
            );
            let mut input = File::open(path)
                .context("deferred: graph original referenced capture unavailable")?;
            input.seek(std::io::SeekFrom::Start(reference.record_offset))?;
            let mut records = crate::warc::RecordIterator::with_limits(
                input.take(reference.record_end - reference.record_offset),
                c.consumer,
            );
            let original = records.next().context("deferred: empty graph original")??;
            let original_meta = original
                .metadata
                .capture
                .as_ref()
                .context("graph reference requires capture metadata")?;
            anyhow::ensure!(
                original_meta.fetch_id == reference.fetch_id
                    && original_meta.payload_sha256 == reference.payload_sha256
                    && original_meta.payload_bytes == reference.payload_bytes
                    && !original.is_revalidation()
                    && capture::url_key(&Url::parse(&original_meta.requested_url)?)? == url,
                "graph original reference integrity/identity mismatch"
            );
            anyhow::ensure!(
                records.next().is_none(),
                "graph reference contains additional records or corrupt trailer"
            );
            process_record(
                ledger,
                c,
                &original,
                &format!("reference:{}", reference.fetch_id),
                0,
            )?;
        }
        let tx = ledger.db.begin_write()?;
        let current = get::<Current>(&tx, CURRENT, &url)?
            .context("deferred: graph original has no current version")?;
        let doc = get::<Document>(&tx, DOCS, &current.version)?
            .context("graph original version missing")?;
        anyhow::ensure!(
            doc.payload_sha256 == reference.payload_sha256 || current.observed_at > at,
            "deferred: graph reference does not match current original"
        );
        if current.observed_at <= at {
            put(
                &tx,
                CURRENT,
                &url,
                &Current {
                    observed_at: at,
                    fetch_id: fetch_id.clone(),
                    ..current.clone()
                },
            )?;
        }
        decision.version = Some(current.version.clone());
        decision.outcome = "revalidated".into();
        decide(&tx, &decision)?;
        tx.open_table(CAPTURES)?
            .insert(fetch_id.as_str(), current.version.as_str())?;
        tx.commit()?;
        return Ok(());
    }
    let status = metadata.map_or(200, |m| m.response.status);
    let nofollow = !record.links_allowed();
    let id = capture::sha256(&serde_json::to_vec(&(
        "graph-extractor-v1",
        &url,
        &final_url,
        &payload,
        status,
        nofollow,
    ))?);
    decision.version = Some(id.clone());
    let bucket =
        (u64::from_str_radix(&capture::sha256(url.as_bytes())[..16], 16)? % c.buckets) as u64;
    let mut doc = Document {
        id: id.clone(),
        url: url.clone(),
        final_url: final_url.clone(),
        fetch_id: fetch_id.clone(),
        captured_at: at,
        payload_sha256: payload,
        source: source.into(),
        ordinal,
        bucket,
        edges: 0,
        host_degree: 0,
        state: "active".into(),
        canonical: None,
        metadata: metadata.cloned(),
    };
    let tx = ledger.db.begin_write()?;
    let existing = get::<Document>(&tx, DOCS, &id)?;
    if let Some(known) = existing {
        doc = known;
        decision.outcome = "duplicate_version".into();
    } else if status == 410 {
        doc.state = "withdrawn".into();
        decision.outcome = "withdrawn".into();
    } else if !(200..300).contains(&status) {
        decision.outcome = "omitted_status".into();
        decision.reason = Some(format!("HTTP {status}; previous graph version retained"));
        decide(&tx, &decision)?;
        tx.commit()?;
        return Ok(());
    } else if record.response.payload_type != Some(crate::warc::PayloadType::Html) {
        decision.outcome = "omitted_mime".into();
        decision.reason = Some("non-HTML capture; previous version retained".into());
        decide(&tx, &decision)?;
        tx.commit()?;
        return Ok(());
    } else if nofollow {
        doc.state = "nofollow".into();
        decision.outcome = "nofollow".into();
    } else {
        let html = {
            let _span = metrics::Span::new(1);
            Html::parse_for_graph(&record.response.body, &record.request.url)?
        };
        doc.canonical = html.canonical_url().map(|v| v.to_string());
        if html.is_no_follow() {
            doc.state = "nofollow".into();
            decision.outcome = "nofollow".into();
        } else {
            let _span = metrics::Span::new(2);
            let mut edges = tx.open_table(EDGES)?;
            let mut hosts = tx.open_table(HOSTS)?;
            let mut bytes = 0usize;
            let mut error = None;
            let mut quota = false;
            let mut visited = 0usize;
            html.visit_anchor_links(|link| {
                visited += 1;
                if visited % 128 == 0 && CANCELLED.load(Ordering::Relaxed) {
                    error = Some(anyhow::anyhow!("cancelled: graph extraction drained"));
                    return false;
                }
                if visited > c.max_links_per_document {
                    quota = true;
                    return false;
                }
                // The acquisition contract admits HTTP(S) resources. Non-fetchable
                // schemes remain in the retained HTML, outside the graph projection.
                if !matches!(link.destination.scheme(), "http" | "https") {
                    return true;
                }
                let parsed = (|| -> Result<StoredEdge> {
                    let destination = capture::url_key(&link.destination)?;
                    let label = link.text.chars().take(128).collect::<String>();
                    let id = capture::sha256(&serde_json::to_vec(&(
                        &final_url,
                        &destination,
                        link.rel.as_u64(),
                        &label,
                    ))?);
                    Ok(StoredEdge {
                        id,
                        source_url: final_url.clone(),
                        destination_url: destination,
                        from: Node::from(html.url()),
                        to: Node::from(link.destination),
                        rel: link.rel,
                        label,
                    })
                })();
                let row = match parsed {
                    Ok(v) => v,
                    Err(e) => {
                        error = Some(e);
                        return false;
                    }
                };
                let result = (|| -> Result<()> {
                    if edges.get((id.as_str(), row.id.as_str()))?.is_none() {
                        let encoded = serde_json::to_vec(&row)?;
                        bytes = bytes.saturating_add(encoded.len());
                        if bytes > c.edge_bytes_per_document {
                            quota = true;
                            return Ok(());
                        }
                        edges.insert((id.as_str(), row.id.as_str()), encoded.as_slice())?;
                        doc.edges += 1;
                    }
                    if let Some(host) = Url::parse(&row.destination_url)?.host_str() {
                        hosts.insert((id.as_str(), host), ())?;
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    error = Some(e);
                    return false;
                }
                !quota
            });
            if let Some(error) = error {
                return Err(error);
            }
            drop(edges);
            drop(hosts);
            if quota {
                drop(tx);
                let tx = ledger.db.begin_write()?;
                decision.outcome = "quota".into();
                decision.reason=Some("document link/byte quota; complete capture retained, previous version unchanged".into());
                decide(&tx, &decision)?;
                tx.commit()?;
                return Ok(());
            }
            doc.host_degree = tx
                .open_table(HOSTS)?
                .range((id.as_str(), "")..=(id.as_str(), "\u{10ffff}"))?
                .count() as u64;
        }
    }
    {
        let _span = metrics::Span::new(3);
        let current = get::<Current>(&tx, CURRENT, &url)?;
        if current.as_ref().is_none_or(|v| v.version != id) {
            if let Some(old) = current {
                let old_doc = get::<Document>(&tx, DOCS, &old.version)?
                    .context("graph current version missing")?;
                loop {
                    let keys = tx
                        .open_table(ACTIVE)?
                        .range(
                            (old_doc.bucket, old.version.as_str(), "")
                                ..=(old_doc.bucket, old.version.as_str(), "\u{10ffff}"),
                        )?
                        .take(128)
                        .map(|v| {
                            v.map(|(k, _)| {
                                (k.value().0, k.value().1.to_owned(), k.value().2.to_owned())
                            })
                        })
                        .collect::<std::result::Result<Vec<_>, _>>()?;
                    if keys.is_empty() {
                        break;
                    }
                    let mut active = tx.open_table(ACTIVE)?;
                    for (b, v, e) in keys {
                        active.remove((b, v.as_str(), e.as_str()))?;
                    }
                }
            }
            let history = tx.open_table(EDGES)?;
            let mut active = tx.open_table(ACTIVE)?;
            for row in history.range((id.as_str(), "")..=(id.as_str(), "\u{10ffff}"))? {
                let (key, value) = row?;
                active.insert((bucket, id.as_str(), key.value().1), value.value())?;
            }
            tx.open_table(DIRTY)?.insert(bucket, ())?;
            let mut meta = tx.open_table(META)?;
            let revision = meta
                .get("revision")?
                .map(|v| v.value().parse::<u64>())
                .transpose()?
                .unwrap_or(0)
                + 1;
            let revision = revision.to_string();
            meta.insert("revision", revision.as_str())?;
        }
        put(&tx, DOCS, &id, &doc)?;
        put(
            &tx,
            CURRENT,
            &url,
            &Current {
                version: id.clone(),
                observed_at: at,
                fetch_id: fetch_id.clone(),
            },
        )?;
        tx.open_table(CAPTURES)?
            .insert(fetch_id.as_str(), id.as_str())?;
        decide(&tx, &decision)?;
    }
    let _span = metrics::Span::new(4);
    failpoint("before_ledger_commit");
    tx.commit()?;
    failpoint("after_ledger_commit");
    Ok(())
}

pub fn run(c: &Config) -> Result<Report> {
    c.validate()?;
    fs::create_dir_all(&c.output_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(c.output_path.join("ingestion.lock"))?;
    lock.try_lock_exclusive()?;
    let ledger = Ledger::open(c)?;
    capacity(c, 0)?;
    let start = Instant::now();
    let cpu = metrics::cpu_ns();
    let mut source_note = Source::default();
    let source = WarcSource::Local(LocalConfig {
        folder: c.source_directory.to_string_lossy().into(),
        names: c.files.clone(),
    });
    for name in &c.files {
        check_control()?;
        let mut consumer = {
            let _span = metrics::Span::new(0);
            Consumer::open(
                &source,
                name,
                &c.output_path.join("checkpoints"),
                c.consumer,
            )?
        };
        let key = consumer.source_key().to_owned();
        source_note = Source {
            name: name.clone(),
            fingerprint: consumer.fingerprint().into(),
            records: consumer.cursor(),
            complete: false,
            error: None,
        };
        loop {
            if consumer.cursor() % 16 == 0 {
                capacity(c, 0)?;
            }
            check_control()?;
            let batch = {
                let _span = metrics::Span::new(0);
                consumer.next_batch(1)
            };
            let batch = match batch {
                Ok(Some(v)) => v,
                Ok(None) => break,
                Err(error) => {
                    source_note.error = Some(error.to_string());
                    let tx = ledger.db.begin_write()?;
                    put(&tx, SOURCES, &key, &source_note)?;
                    tx.commit()?;
                    project(
                        c,
                        &ledger.report(
                            "quarantined",
                            &source_note,
                            start,
                            cpu,
                            Some(error.to_string()),
                        )?,
                    );
                    return Err(error);
                }
            };
            if let Err(error) =
                process_record(&ledger, c, &batch.records[0], &key, batch.next_record)
            {
                source_note.error = Some(error.to_string());
                let tx = ledger.db.begin_write()?;
                put(&tx, SOURCES, &key, &source_note)?;
                tx.commit()?;
                project(
                    c,
                    &ledger.report(
                        "deferred",
                        &source_note,
                        start,
                        cpu,
                        Some(error.to_string()),
                    )?,
                );
                return Err(error);
            }
            source_note.records = batch.next_record;
            let tx = ledger.db.begin_write()?;
            put(&tx, SOURCES, &key, &source_note)?;
            tx.commit()?;
            failpoint("before_cursor");
            consumer.confirm(batch.next_record, false)?;
            failpoint("after_cursor");
            drop(batch);
            release_unused_allocations();
            if consumer.cursor() % 16 == 0 {
                project(
                    c,
                    &ledger.report("extracting", &source_note, start, cpu, None)?,
                );
            }
        }
        source_note.complete = true;
        let tx = ledger.db.begin_write()?;
        put(&tx, SOURCES, &key, &source_note)?;
        tx.commit()?;
        consumer.finish()?;
    }
    if c.publish_generations {
        prepare_with_ledger(c, &ledger, false)?;
    }
    let report = ledger.report("committed", &source_note, start, cpu, None)?;
    project(c, &report);
    Ok(report)
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Partition {
    pub bucket: u64,
    pub hash: String,
    pub edges: u64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Manifest {
    #[serde(default)]
    pub artifacts: BTreeMap<String, String>,
    pub version: u32,
    #[serde(default)]
    pub revision: u64,
    pub id: String,
    pub state: String,
    pub logical_sha256: String,
    pub documents: u64,
    pub edges: u64,
    pub partitions: Vec<Partition>,
    pub reused_buckets: u64,
    pub rebuilt_buckets: u64,
    pub elapsed_ms: f64,
    pub node_identity: String,
    pub exact_identity: String,
    pub scores_state: String,
    pub build_id: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Selector {
    pub version: u32,
    pub active: String,
    pub previous: Option<String>,
    pub manifest_sha256: String,
}
fn read_selector(c: &Config) -> Result<Option<Selector>> {
    let p = c.output_path.join("graph-selector.json");
    if !p.exists() {
        return Ok(None);
    }
    let s: Selector = serde_json::from_reader(File::open(p)?.take(65536))?;
    anyhow::ensure!(s.version == 1, "unknown graph selector version");
    Ok(Some(s))
}
fn read_manifest(c: &Config, id: &str) -> Result<Manifest> {
    uuid::Uuid::parse_str(id)?;
    let m: Manifest = serde_json::from_reader(
        File::open(
            c.output_path
                .join("generations")
                .join(id)
                .join("manifest.json"),
        )?
        .take(1024 * 1024),
    )?;
    anyhow::ensure!(
        m.version == 1 && m.state == "ready" && m.id == id,
        "unknown or partial graph generation"
    );
    Ok(m)
}
fn link_tree(source: &Path, target: &Path) -> Result<()> {
    fs::create_dir_all(target)?;
    for f in fs::read_dir(source)? {
        let f = f?;
        let name = f.file_name();
        let to = target.join(&name);
        if f.file_type()?.is_dir() {
            link_tree(&f.path(), &to)?;
        } else if f.file_type()?.is_file() {
            let name = name.to_string_lossy();
            if name.ends_with(".lock") {
                continue;
            }
            if name == "meta.json" || name == ".managed.json" {
                fs::copy(f.path(), to)?;
            } else {
                fs::hard_link(f.path(), to)?;
            }
        }
    }
    Ok(())
}
fn sync_tree(root: &Path) -> Result<()> {
    for f in fs::read_dir(root)? {
        let f = f?;
        if f.file_type()?.is_dir() {
            sync_tree(&f.path())?;
        } else if f.file_type()?.is_file() {
            File::open(f.path())?.sync_all()?;
        }
    }
    File::open(root)?.sync_all()?;
    Ok(())
}
pub fn prepare(c: &Config, rebuild: bool) -> Result<Manifest> {
    c.validate()?;
    fs::create_dir_all(&c.output_path)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(c.output_path.join("ingestion.lock"))?;
    lock.try_lock_exclusive()?;
    let ledger = Ledger::open(c)?;
    prepare_with_ledger(c, &ledger, rebuild)
}
fn prepare_with_ledger(c: &Config, ledger: &Ledger, rebuild: bool) -> Result<Manifest> {
    check_control()?;
    capacity(c, 0)?;
    repair(c)?;
    let start = Instant::now();
    let read = ledger.db.begin_read()?;
    let current = read.open_table(CURRENT)?;
    let documents = read.open_table(DOCS)?;
    let active = read.open_table(ACTIVE)?;
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut catalog_versions = 0;
    for row in current.iter()? {
        let (url, value) = row?;
        let value: Current = serde_json::from_slice(value.value())?;
        digest.update(url.value().as_bytes());
        digest.update(value.version.as_bytes());
        catalog_versions += 1;
    }
    let logical = capture::hex(digest.finish().as_ref());
    let previous = read_selector(c)?;
    let previous_manifest = previous
        .as_ref()
        .map(|s| read_manifest(c, &s.active))
        .transpose()?;
    if !rebuild {
        if let Some(old) = &previous_manifest {
            if old.logical_sha256 == logical {
                let mut old = old.clone();
                old.reused_buckets = c.buckets;
                old.rebuilt_buckets = 0;
                old.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                return Ok(old);
            }
        }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let root = c.output_path.join("generations").join(&id);
    fs::create_dir_all(&root)?;
    {
        let mut inputs = BufWriter::with_capacity(65536, File::create(root.join("inputs.jsonl"))?);
        for row in read.open_table(SOURCES)?.iter()? {
            let (_, value) = row?;
            inputs.write_all(value.value())?;
            inputs.write_all(b"\n")?;
        }
        inputs.flush()?;
    }
    consumer::atomic_json(&root.join("config.json"), c)?;
    let mut partitions = Vec::new();
    let mut reused = 0;
    let mut rebuilt = 0;
    let mut total = 0;
    let mut catalog = BufWriter::with_capacity(65536, File::create(root.join("catalog.jsonl"))?);
    for row in current.iter()? {
        let (_, v) = row?;
        let v: Current = serde_json::from_slice(v.value())?;
        let doc = documents
            .get(v.version.as_str())?
            .context("graph catalog version missing")?;
        catalog.write_all(doc.value())?;
        catalog.write_all(b"\n")?;
    }
    catalog.flush()?;
    drop(catalog);
    {
        let mut edges = BufWriter::with_capacity(65536, File::create(root.join("edges.jsonl"))?);
        for row in active.iter()? {
            let (key, value) = row?;
            let edge: StoredEdge = serde_json::from_slice(value.value())?;
            serde_json::to_writer(
                &mut edges,
                &serde_json::json!({"version":key.value().1,"edge":edge}),
            )?;
            edges.write_all(b"\n")?;
        }
        edges.flush()?;
    }
    {
        let mut decisions =
            BufWriter::with_capacity(65536, File::create(root.join("decisions.jsonl"))?);
        for row in read.open_table(RECORDS)?.iter()? {
            let (_, value) = row?;
            decisions.write_all(value.value())?;
            decisions.write_all(b"\n")?;
        }
        decisions.flush()?;
    }
    for bucket in 0..c.buckets {
        check_control()?;
        let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
        let mut edges = 0;
        for row in active.range((bucket, "", "")..=(bucket, "\u{10ffff}", "\u{10ffff}"))? {
            let (_, value) = row?;
            hash.update(value.value());
            edges += 1;
        }
        let hash = capture::hex(hash.finish().as_ref());
        let partition = Partition {
            bucket,
            hash: hash.clone(),
            edges,
        };
        total += edges;
        let path = root.join("partitions").join(bucket.to_string());
        let same = !rebuild
            && previous_manifest.as_ref().is_some_and(|m| {
                m.partitions
                    .iter()
                    .any(|p| p.bucket == bucket && p.hash == hash)
            });
        if same {
            let old = c
                .output_path
                .join("generations")
                .join(&previous.as_ref().unwrap().active)
                .join("partitions")
                .join(bucket.to_string());
            link_tree(&old, &path)?;
            reused += 1;
        } else {
            capacity(c, (c.writer_bytes as u64).min(c.temporary_bytes))?;
            let _span = metrics::Span::new(5);
            let mut graph = Webgraph::open(&path, 0u64.into())?;
            graph.prepare_writer_with_budget(c.writer_bytes)?;
            let mut last = String::new();
            let mut degree = 0;
            for row in active.range((bucket, "", "")..=(bucket, "\u{10ffff}", "\u{10ffff}"))? {
                check_control()?;
                let (key, value) = row?;
                if last != key.value().1 {
                    last = key.value().1.into();
                    degree = serde_json::from_slice::<Document>(
                        documents
                            .get(last.as_str())?
                            .context("graph edge version missing")?
                            .value(),
                    )?
                    .host_degree;
                }
                let edge: StoredEdge = serde_json::from_slice(value.value())?;
                graph.insert_versioned(edge.edge(degree))?;
            }
            graph.commit()?;
            rebuilt += 1;
        }
        release_unused_allocations();
        partitions.push(partition);
        anyhow::ensure!(
            tree_bytes(&root)? <= c.temporary_bytes,
            "deferred: graph generation temporary quota reached; previous generation retained"
        );
        consumer::atomic_json(
            &root.join("progress.json"),
            &serde_json::json!({"version":1,"id":id,"stage":"building","buckets_completed":bucket+1,"buckets_total":c.buckets,"edges":total}),
        )?;
        failpoint("during_partition");
    }
    {
        let _span = metrics::Span::new(6);
        let work = root.join("assembly");
        fs::create_dir(&work)?;
        for bucket in 0..c.buckets {
            link_tree(
                &root.join("partitions").join(bucket.to_string()),
                &work.join(bucket.to_string()),
            )?;
        }
        let first = work.join("0");
        let mut graph = Webgraph::open(&first, 0u64.into())?;
        for bucket in 1..c.buckets {
            check_control()?;
            let other = Webgraph::open(work.join(bucket.to_string()), 0u64.into())?;
            graph.merge_committed(other)?;
            release_unused_allocations();
        }
        drop(graph);
        fs::rename(first, root.join("graph"))?;
    }
    {
        let _span = metrics::Span::new(7);
        let graph = Webgraph::open(root.join("graph"), 0u64.into())?;
        anyhow::ensure!(
            graph.page_edges().count() as u64 == total,
            "graph projection count differs from exact ledger"
        );
    }
    let revision = read
        .open_table(META)?
        .get("revision")?
        .map(|v| v.value().parse::<u64>())
        .transpose()?
        .unwrap_or(0);
    let mut manifest = Manifest {
        artifacts: BTreeMap::new(),
        version: 1,
        revision,
        id: id.clone(),
        state: "ready".into(),
        logical_sha256: logical,
        documents: catalog_versions,
        edges: total,
        partitions,
        reused_buckets: reused,
        rebuilt_buckets: rebuilt,
        elapsed_ms: 0.0,
        node_identity:
            "legacy-node-v1 (scheme/www/slash/tracking alias); exact provenance retained".into(),
        exact_identity: "C11 URL v1; source-version/destination/rel/label".into(),
        scores_state: "budget".into(),
        build_id: option_env!("WEBGRAPH_BUILD_ID")
            .unwrap_or("unpackaged")
            .into(),
    };
    if total > 0 && total <= c.max_centrality_edges {
        calculate_scores(c, &root)?;
        manifest.scores_state = "calculated".into();
    } else if total == 0 {
        manifest.scores_state = "empty".into();
    }
    manifest.artifacts = artifact_hashes(&root)?;
    sync_tree(&root)?;
    manifest.elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    failpoint("before_manifest");
    consumer::atomic_json(&root.join("manifest.json"), &manifest)?;
    failpoint("after_manifest");
    let manifest_sha256 = consumer::checksum(File::open(root.join("manifest.json"))?)?;
    let selector = Selector {
        version: 1,
        active: id,
        previous: previous.map(|s| s.active),
        manifest_sha256,
    };
    consumer::atomic_json(
        &c.output_path.join("publication.json"),
        &serde_json::json!({"version":1,"selector":selector,"complete":false}),
    )?;
    failpoint("before_selector");
    consumer::atomic_json(&c.output_path.join("graph-selector.json"), &selector)?;
    failpoint("after_selector");
    consumer::atomic_json(
        &c.output_path.join("publication.json"),
        &serde_json::json!({"version":1,"selector":selector,"complete":true}),
    )?;
    let tx = ledger.db.begin_write()?;
    {
        let keys = tx
            .open_table(DIRTY)?
            .iter()?
            .map(|v| v.map(|(k, _)| k.value()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut dirty = tx.open_table(DIRTY)?;
        for key in keys {
            dirty.remove(key)?;
        }
    }
    tx.commit()?;
    Ok(manifest)
}
fn artifact_hashes(root: &Path) -> Result<BTreeMap<String, String>> {
    let mut hashes = BTreeMap::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for file in fs::read_dir(path)? {
            let file = file?;
            if file.file_type()?.is_dir() {
                stack.push(file.path());
            } else if file.file_type()?.is_file() {
                let name = file.file_name();
                let name = name.to_string_lossy();
                if name.ends_with(".lock")
                    || matches!(name.as_ref(), "manifest.json" | "progress.json")
                {
                    continue;
                }
                let relative = file
                    .path()
                    .strip_prefix(root)?
                    .to_string_lossy()
                    .into_owned();
                hashes.insert(relative, consumer::checksum(File::open(file.path())?)?);
                anyhow::ensure!(
                    hashes.len() <= 100_000,
                    "graph artifact count quota reached"
                );
            }
        }
    }
    Ok(hashes)
}
fn verify_artifacts(root: &Path, manifest: &Manifest) -> Result<()> {
    anyhow::ensure!(
        artifact_hashes(root)? == manifest.artifacts,
        "graph generation artifact checksum mismatch; generation retained for diagnosis"
    );
    Ok(())
}
fn calculate_scores(c: &Config, root: &Path) -> Result<()> {
    let graph = Webgraph::open(root.join("graph"), 0u64.into())?;
    let scores = super::centrality::harmonic::HarmonicCentrality::calculate(&graph);
    let _span = metrics::Span::new(9);
    let folder = root.join("scores");
    fs::create_dir_all(&folder)?;
    let mut values: speedy_kv::Db<super::NodeID, f64> =
        speedy_kv::Db::open_or_create(folder.join("harmonic"))?;
    let mut ranks: speedy_kv::Db<super::NodeID, u64> =
        speedy_kv::Db::open_or_create(folder.join("harmonic_rank"))?;
    let sort: TableDefinition<(u64, &str), f64> = TableDefinition::new("scores.order.v1");
    let db = Database::builder()
        .set_cache_size(c.cache_bytes)
        .create(folder.join("order.redb"))?;
    let tx = db.begin_write()?;
    {
        let mut table = tx.open_table(sort)?;
        for (node, value) in scores.iter() {
            anyhow::ensure!(value.is_finite() && value >= 0.0, "non-finite graph score");
            values.insert(*node, value)?;
            let key = format!("{:032x}", node.as_u128());
            table.insert((u64::MAX - value.to_bits(), key.as_str()), value)?;
        }
    }
    tx.commit()?;
    values.commit()?;
    let read = db.begin_read()?;
    let mut csv = BufWriter::with_capacity(65536, File::create(folder.join("harmonic.csv"))?);
    for (rank, row) in read.open_table(sort)?.iter()?.enumerate() {
        check_control()?;
        let (key, value) = row?;
        let node = super::NodeID::from(u128::from_str_radix(key.value().1, 16)?);
        ranks.insert(node, rank as u64)?;
        if rank < 1000 {
            if let Some(host) = graph.search(&super::query::Id2NodeQuery::Host(node))? {
                writeln!(csv, "{},{}", host.as_str(), value.value())?;
            }
        }
        if rank % 128 == 127 {
            ranks.commit()?;
        }
    }
    ranks.commit()?;
    csv.flush()?;
    Ok(())
}
pub fn repair(c: &Config) -> Result<()> {
    let path = c.output_path.join("publication.json");
    if !path.exists() {
        return Ok(());
    }
    let journal: serde_json::Value = serde_json::from_reader(File::open(&path)?.take(65536))?;
    anyhow::ensure!(journal["version"] == 1, "unknown graph publication journal");
    if journal["complete"] == true {
        return Ok(());
    }
    let selector: Selector = serde_json::from_value(journal["selector"].clone())?;
    let manifest = read_manifest(c, &selector.active)?;
    let root = c.output_path.join("generations").join(&selector.active);
    anyhow::ensure!(
        consumer::checksum(File::open(root.join("manifest.json"))?)? == selector.manifest_sha256,
        "graph manifest checksum mismatch"
    );
    verify_artifacts(&root, &manifest)?;
    consumer::atomic_json(&c.output_path.join("graph-selector.json"), &selector)?;
    consumer::atomic_json(
        &path,
        &serde_json::json!({"version":1,"selector":selector,"complete":true}),
    )?;
    Ok(())
}
pub fn query(c: &Config, url: &str, backlinks: bool) -> Result<serde_json::Value> {
    let selector = read_selector(c)?.context("no selected graph generation")?;
    let manifest = read_manifest(c, &selector.active)?;
    let root = c.output_path.join("generations").join(&manifest.id);
    verify_artifacts(&root, &manifest)?;
    let graph = Webgraph::open(root.join("graph"), 0u64.into())?;
    let node = Node::from(Url::parse(url)?);
    let edges = if backlinks {
        graph.search(
            &super::query::FullBacklinksQuery::new(node)
                .with_limit(super::EdgeLimit::Limit(128))
                .skip_self_links(false)
                .deduplicate(false),
        )?
    } else {
        graph.search(
            &super::query::FullForwardlinksQuery::new(node)
                .with_limit(super::EdgeLimit::Limit(128))
                .skip_self_links(false)
                .deduplicate(false),
        )?
    };
    Ok(serde_json::json!({"generation":manifest.id,"edges":edges}))
}
pub fn audit(c: &Config) -> Result<serde_json::Value> {
    let ledger = Ledger::open(c)?;
    let tx = ledger.db.begin_read()?;
    let mut decisions = BTreeMap::new();
    for row in tx.open_table(COUNTS)?.iter()? {
        let (k, v) = row?;
        decisions.insert(k.value().to_owned(), v.value());
    }
    let selected = read_selector(c)?;
    let manifest = selected
        .as_ref()
        .map(|s| read_manifest(c, &s.active))
        .transpose()?;
    Ok(
        serde_json::json!({"version":1,"documents":tx.open_table(CURRENT)?.len()?,"current_edges":tx.open_table(ACTIVE)?.len()?,"stored_versions":tx.open_table(DOCS)?.len()?,"records":tx.open_table(RECORDS)?.len()?,"decisions":decisions,"selector":selected,"manifest":manifest}),
    )
}
