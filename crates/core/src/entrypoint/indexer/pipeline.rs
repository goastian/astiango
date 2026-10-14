use super::{
    metrics,
    worker::{IndexingWorker, TextCache},
    IndexableWebpage, JobSettings,
};
use crate::{
    config::IndexerConfig,
    index::Index,
    warc::{consumer::Consumer, PayloadType, WarcRecord},
};
use anyhow::{Context, Result};
use fs4::FileExt;
use redb::{Database, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, Read, Write},
    path::{Path, PathBuf},
    time::Instant,
};

pub static CANCELLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub mod generations;
const SOURCES: TableDefinition<&str, &[u8]> = TableDefinition::new("sources.v1");
const DECISIONS: TableDefinition<&str, &[u8]> = TableDefinition::new("decisions.v1");
const CURRENT: TableDefinition<&str, &[u8]> = TableDefinition::new("current.v1");
const PAYLOADS: TableDefinition<&str, ()> = TableDefinition::new("payloads.v1");
const COUNTS: TableDefinition<&str, u64> = TableDefinition::new("counts.v1");
const VERSIONS: TableDefinition<&str, ()> = TableDefinition::new("versions.v1");
const OUTBOX: TableDefinition<&str, &[u8]> = TableDefinition::new("outbox.v1");

#[derive(Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub writer_bytes: usize,
    pub memory_budget_bytes: usize,
    pub cache_bytes: usize,
    pub model_bytes: usize,
    pub commit_records: usize,
    pub prepared_bytes: usize,
    pub max_segments: u64,
    pub merge_temporary_bytes: u64,
    pub max_output_bytes: u64,
    pub disk_reserve_bytes: u64,
    pub report_path: Option<PathBuf>,
    pub derivatives: Vec<PathBuf>,
    pub publish_generations: bool,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            writer_bytes: 24 * 1024 * 1024,
            memory_budget_bytes: 512 * 1024 * 1024,
            cache_bytes: 4 * 1024 * 1024,
            model_bytes: 0,
            commit_records: 64,
            prepared_bytes: 32 * 1024 * 1024,
            max_segments: 8,
            merge_temporary_bytes: 256 * 1024 * 1024,
            max_output_bytes: 2 * 1024 * 1024 * 1024,
            disk_reserve_bytes: 256 * 1024 * 1024,
            report_path: None,
            derivatives: vec![],
            publish_generations: false,
        }
    }
}
impl Config {
    pub fn validate(&self, base: &IndexerConfig) -> Result<()> {
        base.consumer.validate()?;
        anyhow::ensure!(
            (base.dual_encoder.is_none() && base.safety_classifier_path.is_none())
                || self.model_bytes > 0,
            "configured models require an explicit memory reservation"
        );
        base.dedup.validate()?;
        anyhow::ensure!(
            (15 * 1024 * 1024..=128 * 1024 * 1024).contains(&self.writer_bytes),
            "writer budget must be 15..128 MiB"
        );
        anyhow::ensure!(
            (1..=128).contains(&self.commit_records)
                && (1..=64).contains(&base.batch_size)
                && self.max_segments > 0
                && self.max_segments <= 64,
            "invalid commit, batch or merge limits"
        );
        let reserve = base
            .consumer
            .batch_bytes
            .checked_add(
                base.consumer
                    .decoded_bytes
                    .checked_mul(4)
                    .context("DOM budget overflow")?,
            )
            .and_then(|n| n.checked_add(self.writer_bytes))
            .and_then(|n| n.checked_add(self.cache_bytes))
            .and_then(|n| n.checked_add(self.model_bytes))
            .and_then(|n| n.checked_add(self.prepared_bytes.checked_mul(2)?))
            .and_then(|n| n.checked_add(8 * 1024 * 1024))
            .context("index memory budget overflow")?;
        anyhow::ensure!(
            reserve <= self.memory_budget_bytes
                && self.cache_bytes <= 32 * 1024 * 1024
                && self.prepared_bytes >= 65536,
            "combined reader/DOM/writer/cache budget exceeds memory profile"
        );
        anyhow::ensure!(
            self.max_output_bytes >= self.writer_bytes as u64 * 4 && self.derivatives.len() <= 32,
            "invalid output or derivative budget"
        );
        Ok(())
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Decision {
    pub id: String,
    pub source: String,
    pub record: u64,
    pub capture_id: Option<String>,
    pub url: String,
    pub version: String,
    pub captured_at_ms: i64,
    pub payload_sha256: String,
    pub kind: String,
    pub outcome: String,
    pub planned: String,
    pub reason: String,
    pub prior_payload: Option<String>,
    #[serde(default)]
    pub indexed_text_sha256: Option<String>,
    #[serde(default)]
    pub indexed_document_sha256: Option<String>,
    pub metadata: Option<crate::capture::Metadata>,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub source_reference: Option<crate::crawler::recrawl::PayloadReference>,
}
struct Ledger {
    db: Database,
}
impl Ledger {
    fn open(path: &Path) -> Result<Self> {
        let db = Database::builder()
            .set_cache_size(4 * 1024 * 1024)
            .create(path)?;
        {
            let tx = db.begin_write()?;
            tx.open_table(SOURCES)?;
            tx.open_table(DECISIONS)?;
            tx.open_table(CURRENT)?;
            tx.open_table(PAYLOADS)?;
            tx.open_table(COUNTS)?;
            tx.open_table(OUTBOX)?;
            tx.open_table(VERSIONS)?;
            tx.open_table(generations::PREPARED)?;
            tx.open_table(generations::INPUTS)?;
            tx.open_table(generations::CONTEXTS)?;
            tx.open_table(generations::PREPARED_CONTEXTS)?;
            tx.commit()?;
        }
        // The original decision can be noindex or an unsupported document. A 304
        // references that durable capture even when it has no searchable document.
        if db
            .begin_read()?
            .open_table(PAYLOADS)?
            .get("decision-authority-v1")?
            .is_none()
        {
            let read = db.begin_read()?;
            let tx = db.begin_write()?;
            {
                let mut payloads = tx.open_table(PAYLOADS)?;
                for entry in read.open_table(DECISIONS)?.iter()? {
                    let (_, value) = entry?;
                    let row: Decision = serde_json::from_slice(value.value())?;
                    if ["indexed", "noindex", "excluded"].contains(&row.outcome.as_str())
                        && row.prior_payload.is_none()
                    {
                        payloads
                            .insert(format!("{}:{}", row.url, row.payload_sha256).as_str(), ())?;
                    }
                }
                payloads.insert("decision-authority-v1", ())?;
            }
            tx.commit()?;
        }
        Ok(Self { db })
    }
    fn source(&self, id: &str, outcome: &str, error: Option<&str>) -> Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(SOURCES)?;
            let bytes = serde_json::to_vec(
                &serde_json::json!({"version":1,"outcome":outcome,"error":error,"updated_at":chrono::Utc::now()}),
            )?;
            table.insert(id, bytes.as_slice())?;
        }
        tx.commit()?;
        Ok(())
    }
    fn current(&self, url: &str) -> Result<Option<Decision>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(CURRENT)?;
        let result = table
            .get(url)?
            .map(|value| serde_json::from_slice(value.value()))
            .transpose()?;
        Ok(result)
    }
    fn version_known(&self, version: &str) -> Result<bool> {
        Ok(self
            .db
            .begin_read()?
            .open_table(VERSIONS)?
            .get(version)?
            .is_some())
    }
    fn decided(&self, id: &str) -> Result<bool> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(DECISIONS)?;
        Ok(table
            .get(id)?
            .map(|v| serde_json::from_slice::<Decision>(v.value()))
            .transpose()?
            .is_some_and(|row| row.outcome != "pending"))
    }
    fn documents(&self) -> Result<u64> {
        Ok(self
            .db
            .begin_read()?
            .open_table(COUNTS)?
            .get("active_documents")?
            .map_or(0, |v| v.value()))
    }
    fn payload_known(&self, url: &str, digest: &str) -> Result<bool> {
        Ok(self
            .db
            .begin_read()?
            .open_table(PAYLOADS)?
            .get(format!("{url}:{digest}").as_str())?
            .is_some())
    }
    fn plan(&self, rows: &[Decision]) -> Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut table = tx.open_table(DECISIONS)?;
            let mut counts = tx.open_table(COUNTS)?;
            for row in rows {
                if let Some(previous) = table.get(row.id.as_str())? {
                    let previous: Decision = serde_json::from_slice(previous.value())?;
                    if previous.outcome != "pending" {
                        continue;
                    }
                } else {
                    let count = counts.get("pending")?.map_or(0, |v| v.value());
                    counts.insert("pending", count + 1)?;
                }
                let encoded = serde_json::to_vec(row)?;
                table.insert(row.id.as_str(), encoded.as_slice())?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    fn apply(&self, payload: &str) -> Result<()> {
        let value: serde_json::Value = serde_json::from_str(payload)?;
        anyhow::ensure!(
            value["pipeline_version"] == 1,
            "unknown indexing ledger/engine payload version"
        );
        let ids = value["decision_ids"]
            .as_array()
            .context("missing decision IDs")?;
        anyhow::ensure!(ids.len() <= 128, "commit envelope exceeds decision budget");
        let tx = self.db.begin_write()?;
        {
            let mut decisions = tx.open_table(DECISIONS)?;
            let mut current = tx.open_table(CURRENT)?;
            let mut counts = tx.open_table(COUNTS)?;
            let mut versions = tx.open_table(VERSIONS)?;
            let mut payloads = tx.open_table(PAYLOADS)?;
            for id in ids {
                let id = id.as_str().context("invalid decision ID")?;
                let mut row: Decision = {
                    let saved = decisions.get(id)?.context(
                        "engine commit has no planned decision; retain index for reconciliation",
                    )?;
                    serde_json::from_slice(saved.value())?
                };
                if row.outcome != "pending" {
                    continue;
                }
                row.outcome = row.planned.clone();
                let pending = counts.get("pending")?.map_or(0, |v| v.value());
                counts.insert("pending", pending.saturating_sub(1))?;
                let count = counts.get(row.outcome.as_str())?.map_or(0, |v| v.value());
                counts.insert(row.outcome.as_str(), count + 1)?;
                if row.source_reference.is_some() {
                    let count = counts.get("reference_inputs")?.map_or(0, |v| v.value());
                    counts.insert("reference_inputs", count + 1)?;
                }
                if ["indexed", "noindex", "excluded"].contains(&row.outcome.as_str())
                    && row.prior_payload.is_none()
                {
                    payloads.insert(format!("{}:{}", row.url, row.payload_sha256).as_str(), ())?;
                }
                if row.outcome == "indexed" || row.outcome == "noindex" {
                    let previous = current
                        .get(row.url.as_str())?
                        .map(|v| serde_json::from_slice::<Decision>(v.value()))
                        .transpose()?;
                    let active = counts.get("active_documents")?.map_or(0, |v| v.value());
                    let active = active.saturating_sub(
                        previous.as_ref().is_some_and(|r| r.outcome == "indexed") as u64,
                    ) + (row.outcome == "indexed") as u64;
                    counts.insert("active_documents", active)?;
                    let encoded = serde_json::to_vec(&row)?;
                    current.insert(row.url.as_str(), encoded.as_slice())?;
                    if row.outcome == "indexed" {
                        versions.insert(row.version.as_str(), ())?;
                        payloads
                            .insert(format!("{}:{}", row.url, row.payload_sha256).as_str(), ())?;
                    }
                }
                let encoded = serde_json::to_vec(&row)?;
                decisions.insert(id, encoded.as_slice())?;
            }
            let mut outbox = tx.open_table(OUTBOX)?;
            let event = serde_json::json!({"version":1,"state":"index_committed","source":value["fingerprint"],"cursor":value["records_committed"],"decision_ids":ids,"consumer_contract":"ledger-current-v1"});
            let bytes = serde_json::to_vec(&event)?;
            outbox.insert(
                crate::capture::sha256(payload.as_bytes()).as_str(),
                bytes.as_slice(),
            )?;
        }
        tx.commit()?;
        Ok(())
    }
    fn counts(&self) -> Result<BTreeMap<String, u64>> {
        let tx = self.db.begin_read()?;
        let table = tx.open_table(COUNTS)?;
        table
            .iter()?
            .filter_map(|row| match row {
                Ok((key, value))
                    if !["active_documents", "reference_inputs"].contains(&key.value()) =>
                {
                    Some(Ok((key.value().into(), value.value())))
                }
                Ok(_) => None,
                Err(e) => Some(Err(e.into())),
            })
            .collect()
    }
}

#[derive(Serialize)]
pub struct Report {
    pub version: u32,
    pub stage: String,
    pub current_url: Option<String>,
    pub documents: u64,
    pub stored_versions: u64,
    pub recovered_reference_inputs: u64,
    pub outcomes: BTreeMap<String, u64>,
    pub stages: BTreeMap<String, metrics::Measurement>,
    pub cache_hits: u64,
    pub elapsed_ms: f64,
    pub cpu_ms: f64,
    pub allocated_bytes: u64,
    pub source_records: u64,
    pub source_complete: bool,
    pub error: Option<String>,
    pub working_index: PathBuf,
}
fn project(path: Option<&Path>, report: &Report) {
    if let Some(path) = path {
        let result: Result<()> = (|| {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let temporary = path.with_extension("tmp");
            let mut out = std::io::BufWriter::new(File::create(&temporary)?);
            serde_json::to_writer(&mut out, report)?;
            out.flush()?;
            drop(out);
            fs::rename(temporary, path)?;
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(%error,"indexing observation unavailable");
        }
    }
}
fn failpoint(name: &str) {
    if std::env::var("INDEXER_FAILPOINT").ok().as_deref() == Some(name) {
        unsafe {
            libc::kill(libc::getpid(), libc::SIGKILL);
        }
    }
}
fn tree_bytes(root: &Path) -> Result<u64> {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for file in fs::read_dir(path)? {
            let file = file?;
            let kind = file.file_type()?;
            if kind.is_dir() {
                stack.push(file.path());
            } else if kind.is_file() {
                total = total.saturating_add(file.metadata()?.len());
            }
        }
    }
    Ok(total)
}
fn capacity(root: &Path, policy: &Config) -> Result<()> {
    let total = tree_bytes(root)?;
    anyhow::ensure!(
        total < policy.max_output_bytes,
        "local index output quota reached; admission deferred"
    );
    let path = std::ffi::CString::new(root.as_os_str().as_encoded_bytes())?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    anyhow::ensure!(
        unsafe { libc::statvfs(path.as_ptr(), &mut stat) } == 0,
        "local disk probe failed"
    );
    anyhow::ensure!(
        stat.f_bavail as u64 * stat.f_frsize as u64 >= policy.disk_reserve_bytes,
        "local disk reserve reached; admission deferred"
    );
    Ok(())
}
fn bounded_merge(index: &mut Index, root: &Path, policy: &Config) -> Result<()> {
    let folder = root.join("working/inverted_index");
    let meta: serde_json::Value =
        serde_json::from_reader(File::open(folder.join("meta.json"))?.take(2 * 1024 * 1024))?;
    let segments = meta["segments"]
        .as_array()
        .context("missing index segments")?;
    if segments.len() <= policy.max_segments as usize {
        return Ok(());
    }
    // One merge runs at a time. Reserve twice existing index bytes plus metadata slack.
    let required = tree_bytes(&folder)?
        .saturating_mul(2)
        .saturating_add(16 * 1024 * 1024);
    anyhow::ensure!(
        required <= policy.merge_temporary_bytes,
        "merge temporary quota reached; confirmed index retained, admission deferred"
    );
    anyhow::ensure!(
        tree_bytes(root)?.saturating_add(required) < policy.max_output_bytes,
        "merge output reserve reached; confirmed index retained, admission deferred"
    );
    let mut merge_policy = policy.clone();
    merge_policy.disk_reserve_bytes = merge_policy.disk_reserve_bytes.saturating_add(required);
    capacity(root, &merge_policy)?;
    let _span = metrics::Span::new(6);
    index
        .inverted_index
        .merge_into_max_segments(policy.max_segments)?;
    Ok(())
}
fn decision(record: &WarcRecord, source: &str, ordinal: u64, kind: &str) -> Result<Decision> {
    let url = crate::capture::url_key(&url::Url::parse(&record.request.url)?)?;
    let version = format!(
        "{}:{kind}",
        crate::crawler::dedup::ExactSet::version_key(record)?
    );
    let meta = record.metadata.capture.clone();
    Ok(Decision {
        id: crate::capture::sha256(format!("{source}:{ordinal}").as_bytes()),
        source: source.into(),
        record: ordinal,
        capture_id: meta.as_ref().map(|m| m.fetch_id.clone()),
        url,
        version,
        captured_at_ms: meta
            .as_ref()
            .map(|m| m.response.received_at.timestamp_millis())
            .or_else(|| record.request.date.map(|d| d.timestamp_millis()))
            .unwrap_or(0),
        payload_sha256: meta
            .as_ref()
            .map(|m| m.payload_sha256.clone())
            .unwrap_or_else(|| crate::capture::sha256(record.response.body.as_bytes())),
        indexed_text_sha256: None,
        indexed_document_sha256: None,
        kind: kind.into(),
        outcome: "pending".into(),
        planned: "indexed".into(),
        reason: String::new(),
        prior_payload: meta
            .as_ref()
            .and_then(|m| m.revalidation.as_ref().map(|r| r.payload_sha256.clone())),
        metadata: meta,
        dependencies: vec![],
        source_reference: None,
    })
}
fn commit(
    index: &mut Index,
    ledger: &Ledger,
    consumer: Option<&mut Consumer>,
    rows: &mut Vec<Decision>,
    cursor: u64,
    complete: bool,
) -> Result<()> {
    if rows.is_empty() && !complete {
        return Ok(());
    }
    ledger.plan(rows)?;
    failpoint("planned");
    let mut value: serde_json::Value = if let Some(consumer) = consumer.as_deref() {
        serde_json::from_str(&consumer.payload(cursor, complete, vec![])?)?
    } else {
        serde_json::json!({"consumer_version":1,"fingerprint":"derived","records_committed":cursor,"complete":complete,"keys":[]})
    };
    value["pipeline_version"] = 1.into();
    if let Some(consumer) = consumer.as_deref() {
        value["input_key"] = consumer.source_key().into();
    }
    value["decision_ids"] = serde_json::to_value(
        rows.iter()
            .flat_map(|r| {
                r.dependencies
                    .iter()
                    .cloned()
                    .chain(std::iter::once(r.id.clone()))
            })
            .collect::<Vec<_>>(),
    )?;
    anyhow::ensure!(
        value["decision_ids"]
            .as_array()
            .is_some_and(|ids| ids.len() <= 128),
        "commit exceeds decision envelope budget"
    );
    let payload = serde_json::to_string(&value)?;
    {
        let _span = metrics::Span::new(5);
        index.commit_with_payload(&payload)?;
    }
    failpoint("engine");
    ledger.apply(&payload)?;
    failpoint("ledger");
    if let Some(consumer) = consumer {
        consumer.confirm(cursor, complete)?;
    }
    failpoint("cursor");
    rows.clear();
    Ok(())
}
pub(super) fn read_reference(
    source: &crate::config::WarcSource,
    reference: &crate::crawler::recrawl::PayloadReference,
    limits: crate::warc::stream::Limits,
) -> Result<(WarcRecord, String, String)> {
    use std::io::{Seek, SeekFrom};
    anyhow::ensure!(
        reference.version == 1
            && reference.segment.len() <= 256
            && reference.segment.ends_with(".warc.gz")
            && Path::new(&reference.segment).components().count() == 1
            && Path::new(&reference.segment)
                .components()
                .all(|p| matches!(p, std::path::Component::Normal(_))),
        "unsafe/unknown original capture reference"
    );
    let crate::config::WarcSource::Local(local) = source else {
        anyhow::bail!(
            "referenced body requires a local retained capture; remote acquisition deferred"
        )
    };
    let directory = Path::new(&local.folder).canonicalize()?;
    let path = directory.join(&reference.segment).canonicalize()?;
    anyhow::ensure!(
        path.parent() == Some(directory.as_path()),
        "capture reference escapes source directory"
    );
    let mut file = File::open(&path)?;
    anyhow::ensure!(
        reference.record_end > reference.record_offset
            && reference.record_end <= file.metadata()?.len()
            && reference.record_end - reference.record_offset <= limits.record_bytes as u64 * 2,
        "capture reference byte range exceeds budget"
    );
    file.seek(SeekFrom::Start(reference.record_offset))?;
    let member_sha = crate::warc::consumer::checksum(
        (&mut file).take(reference.record_end - reference.record_offset),
    )?;
    file.seek(SeekFrom::Start(reference.record_offset))?;
    let input =
        Box::new(file.take(reference.record_end - reference.record_offset)) as Box<dyn Read>;
    let mut records = crate::warc::RecordIterator::with_limits(input, limits);
    let record = records
        .next()
        .context("original capture reference is empty")??;
    let meta = record
        .metadata
        .capture
        .as_ref()
        .context("original capture has no identity metadata")?;
    anyhow::ensure!(
        meta.fetch_id == reference.fetch_id
            && meta.payload_sha256 == reference.payload_sha256
            && crate::capture::url_key(&url::Url::parse(&record.request.url)?)?
                == crate::capture::url_key(&url::Url::parse(&reference.url)?)?
            && !record.is_revalidation(),
        "original capture identity/digest mismatch or recursive reference"
    );
    let identity = format!(
        "reference-v1:{}:{member_sha}",
        crate::warc::publication::Workspace::key(source, &reference.segment)?
    );
    Ok((record, identity, member_sha))
}

async fn ingest_record(
    record: WarcRecord,
    mut row: Decision,
    index: &mut Index,
    ledger: &Ledger,
    staged: &mut HashMap<String, Decision>,
    worker: &IndexingWorker,
    cache: &mut TextCache,
    staged_versions: &mut std::collections::HashSet<String>,
    text: Option<String>,
    source: &crate::config::WarcSource,
) -> Result<(Decision, Option<tantivy::schema::TantivyDocument>)> {
    let mut document = None;
    let prior = staged.get(&row.url).cloned().or(ledger.current(&row.url)?);
    if prior
        .as_ref()
        .is_some_and(|p| p.captured_at_ms > row.captured_at_ms)
    {
        row.planned = "excluded".into();
        row.reason = "older_capture".into();
    } else if !record.indexing_allowed() {
        row.planned = "noindex".into();
        row.reason = "robots_directive".into();
        staged.insert(row.url.clone(), row.clone());
    } else if record.is_revalidation() {
        let mut known = if let Some(hash) = &row.prior_payload {
            ledger.payload_known(&row.url, hash)?
                || staged.values().any(|r| {
                    r.url == row.url
                        && &r.payload_sha256 == hash
                        && ["indexed", "noindex", "excluded"].contains(&r.planned.as_str())
                })
        } else {
            false
        };
        if !known {
            if let Some(reference) = record
                .metadata
                .capture
                .as_ref()
                .and_then(|m| m.revalidation.as_ref())
            {
                match read_reference(source, reference, worker.consumer) {
                    Ok((original, identity, member_sha)) => {
                        let mut recovered = decision(&original, &identity, 0, "html-v1")?;
                        recovered.id = crate::capture::sha256(
                            format!("{}:offset:{}", identity, reference.record_offset).as_bytes(),
                        );
                        recovered.source_reference = Some(reference.clone());
                        if !ledger.decided(&recovered.id)? {
                            let (recovered, prepared) = Box::pin(ingest_record(
                                original,
                                recovered,
                                index,
                                ledger,
                                staged,
                                worker,
                                cache,
                                staged_versions,
                                None,
                                source,
                            ))
                            .await?;
                            anyhow::ensure!(
                                recovered.dependencies.is_empty(),
                                "recursive revalidation reference rejected"
                            );
                            ledger.plan(std::slice::from_ref(&recovered))?;
                            row.dependencies.push(recovered.id);
                            document = prepared;
                        }
                        let tx = ledger.db.begin_write()?;
                        {
                            let bytes = serde_json::to_vec(
                                &serde_json::json!({"version":1,"kind":"warc_record_reference","reference":reference,"member_sha256":member_sha}),
                            )?;
                            tx.open_table(generations::INPUTS)?
                                .insert(identity.as_str(), bytes.as_slice())?;
                        }
                        tx.commit()?;
                        known = true;
                    }
                    Err(error) => {
                        row.reason = format!("reference_unavailable: {error}");
                        ledger.plan(&[row])?;
                        anyhow::bail!("deferred: original capture reference unavailable: {error}");
                    }
                }
            }
        }
        row.planned = if known { "revalidated" } else { "deferred" }.into();
        row.reason = if known {
            "prior_payload_reference"
        } else {
            "prior_payload_not_indexed"
        }
        .into();
        if !known {
            ledger.plan(&[row])?;
            anyhow::bail!("deferred: referenced payload is not indexed; retry after admitting the original capture");
        }
    } else if prior.as_ref().is_some_and(|p| {
        p.version == row.version && (p.outcome == "indexed" || p.planned == "indexed")
    }) {
        row.planned = "duplicate".into();
        row.reason = "exact_current_version".into();
    } else if record
        .response
        .payload_type
        .is_some_and(|p| p != PayloadType::Html)
    {
        row.planned = "excluded".into();
        row.reason = "unsupported_payload_type".into();
    } else {
        match worker
            .prepare_one_with_text(&IndexableWebpage::from(record), cache, text)
            .await
        {
            Ok(page) => {
                row.indexed_text_sha256 = Some(crate::capture::sha256(
                    page.html.clean_text().map_or("", String::as_str).as_bytes(),
                ));
                row.indexed_document_sha256 = Some(document_identity(
                    &row.url,
                    &page.html.title().unwrap_or_default(),
                    page.html.clean_text().map_or("", String::as_str),
                ));
                if !ledger.version_known(&row.version)?
                    && staged_versions.insert(row.version.clone())
                {
                    document = Some(index.inverted_index.prepare_document(&page)?);
                } else {
                    row.reason = "reuse_stored_version".into();
                }
                staged.insert(row.url.clone(), row.clone());
            }
            Err(error) => {
                let reason = error.to_string();
                if [
                    "noindex",
                    "empty title",
                    "empty all text",
                    "too few clean words",
                    "low host_centrality value",
                ]
                .iter()
                .any(|item| reason == *item)
                {
                    row.planned = if reason == "noindex" {
                        "noindex"
                    } else {
                        "excluded"
                    }
                    .into();
                    if reason == "noindex" {
                        staged.insert(row.url.clone(), row.clone());
                    }
                    row.reason = reason;
                } else {
                    row.planned = "deferred".into();
                    row.reason = format!("preparation_failed: {reason}");
                    ledger.plan(&[row])?;
                    anyhow::bail!("recoverable preparation failure: {reason}");
                }
            }
        }
    }
    // The DOM and Webpage have been dropped before serializing replay data.
    if row.planned == "indexed" {
        if let Some(doc) = &document {
            generations::save_prepared(ledger, &row, doc)?;
        }
    }
    Ok((row, document))
}

pub fn run(base: &IndexerConfig, policy: &Config) -> Result<Report> {
    policy.validate(base)?;
    let root = Path::new(&base.output_path);
    fs::create_dir_all(root)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("ingestion.lock"))?;
    lock.try_lock_exclusive()
        .context("another ingestion worker owns this index")?;
    capacity(root, policy)?;
    let started = Instant::now();
    let cpu = metrics::cpu_ns();
    let allocated = metrics::ALLOCATED.load(std::sync::atomic::Ordering::Relaxed);
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut worker = rt.block_on(IndexingWorker::try_new(base.clone().into()))?;
    worker.set_job_settings(JobSettings {
        host_centrality_threshold: base.host_centrality_threshold,
        minimum_clean_words: base.minimum_clean_words,
        batch_size: base.batch_size,
        autocommit_after_num_inserts: policy.commit_records,
    });
    let working = root.join("working");
    let mut index = Index::open(&working)?;
    index
        .inverted_index
        .prepare_writer_with_budget(policy.writer_bytes)?;
    let ledger = Ledger::open(&root.join("ledger.redb"))?;
    generations::initialize_context(&ledger, base)?;
    if let Some(payload) = index.inverted_index.commit_payload()? {
        ledger.apply(&payload)?;
    }
    let mut cache = TextCache::new(policy.cache_bytes);
    let mut report = Report {
        version: 1,
        stage: "read_decode".into(),
        current_url: None,
        documents: ledger.documents()?,
        stored_versions: index.inverted_index.num_documents(),
        recovered_reference_inputs: 0,
        outcomes: ledger.counts()?,
        stages: metrics::snapshot(),
        cache_hits: 0,
        elapsed_ms: 0.,
        cpu_ms: 0.,
        allocated_bytes: 0,
        source_records: 0,
        source_complete: false,
        error: None,
        working_index: working,
    };
    let mut active_source = String::new();
    let mut last_project = Instant::now() - std::time::Duration::from_secs(2);
    let result: Result<()> = (|| {
        for path in base
            .warc_source
            .paths()?
            .into_iter()
            .skip(base.skip_warc_files.unwrap_or(0))
            .take(base.limit_warc_files.unwrap_or(usize::MAX))
        {
            active_source = path.clone();
            ledger.source(&active_source, "pending", None)?;
            capacity(root, policy)?;
            let key = crate::warc::publication::Workspace::key(&base.warc_source, &path)?;
            let mut consumer = Consumer::open(
                &base.warc_source,
                &path,
                &root.join("checkpoints"),
                base.consumer,
            )?;
            if let Some(payload) = index.inverted_index.commit_payload()? {
                let value: serde_json::Value = serde_json::from_str(&payload)?;
                if value["fingerprint"].as_str() == Some(consumer.fingerprint())
                    && value["input_key"].as_str() == Some(consumer.source_key())
                {
                    consumer.recover_commit(Some(&payload))?;
                }
            }
            let source = format!("{key}:{}", consumer.fingerprint());
            generations::save_input(&ledger, &source, &base.warc_source, &path)?;
            let mut rows: Vec<Decision> = Vec::new();
            let mut staged = HashMap::new();
            let mut staged_versions = std::collections::HashSet::new();
            let mut queued_bytes = 0usize;
            loop {
                let next = {
                    let _span = metrics::Span::new(0);
                    consumer
                        .next_batch(base.batch_size)
                        .context("quarantined: WARC read/decode failed")?
                };
                let Some(batch) = next else { break };
                let count = batch.records.len() as u64;
                let first = batch.next_record - count;
                let planned = batch
                    .records
                    .iter()
                    .enumerate()
                    .map(|(i, record)| decision(record, &source, first + i as u64 + 1, "html-v1"))
                    .collect::<Result<Vec<_>>>()?;
                ledger.plan(&planned)?;
                for (record, row) in batch.records.into_iter().zip(planned) {
                    anyhow::ensure!(
                        !CANCELLED.load(std::sync::atomic::Ordering::Relaxed),
                        "cancelled: admission drained; source and pending decisions retained"
                    );
                    if let Ok(delay) = std::env::var("INDEXER_STAGE_DELAY_MS") {
                        let delay: u64 = delay.parse().context("invalid integration delay")?;
                        anyhow::ensure!(delay <= 5000, "integration delay exceeds bound");
                        for _ in 0..delay.div_ceil(20) {
                            std::thread::sleep(std::time::Duration::from_millis(20));
                            anyhow::ensure!(
                                !CANCELLED.load(std::sync::atomic::Ordering::Relaxed),
                                "cancelled: bounded preparation drained"
                            );
                        }
                    }
                    let ordinal = row.record;
                    if ledger.decided(&row.id)? {
                        continue;
                    }
                    report.current_url = Some(row.url.clone());
                    report.stage = "prepare".into();
                    let (row, document) = rt.block_on(ingest_record(
                        record,
                        row,
                        &mut index,
                        &ledger,
                        &mut staged,
                        &worker,
                        &mut cache,
                        &mut staged_versions,
                        None,
                        &base.warc_source,
                    ))?;
                    if rows.iter().map(|r| 1 + r.dependencies.len()).sum::<usize>()
                        + 1
                        + row.dependencies.len()
                        > 128
                    {
                        commit(
                            &mut index,
                            &ledger,
                            Some(&mut consumer),
                            &mut rows,
                            ordinal - 1,
                            false,
                        )?;
                        queued_bytes = 0;
                        staged.retain(|_, saved| {
                            saved.id == row.id || row.dependencies.contains(&saved.id)
                        });
                        staged_versions =
                            staged.values().map(|saved| saved.version.clone()).collect();
                    }
                    if let Some(document) = document {
                        let bytes = document.node_data.capacity() + 4096;
                        anyhow::ensure!(
                            bytes <= policy.prepared_bytes,
                            "prepared document exceeds byte quota; source/ledger retained"
                        );
                        if queued_bytes + bytes > policy.prepared_bytes {
                            commit(
                                &mut index,
                                &ledger,
                                Some(&mut consumer),
                                &mut rows,
                                ordinal - 1,
                                false,
                            )?;
                            queued_bytes = 0;
                            staged.retain(|_, saved| {
                                saved.id == row.id || row.dependencies.contains(&saved.id)
                            });
                            staged_versions.clear();
                            staged_versions.insert(row.version.clone());
                        }
                        index.inverted_index.add_prepared_document(document)?;
                        queued_bytes += bytes;
                    }
                    rows.push(row);
                    report.source_records += 1;
                    if rows.len() >= policy.commit_records {
                        report.stage = "commit".into();
                        commit(
                            &mut index,
                            &ledger,
                            Some(&mut consumer),
                            &mut rows,
                            ordinal,
                            false,
                        )?;
                        staged.clear();
                        staged_versions.clear();
                        queued_bytes = 0;
                        capacity(root, policy)?;
                        bounded_merge(&mut index, root, policy)?;
                    }
                    if last_project.elapsed().as_secs_f64() >= 1. {
                        report.documents = ledger.documents()?;
                        report.stored_versions = index.inverted_index.num_documents();
                        report.outcomes = ledger.counts()?;
                        report.stages = metrics::snapshot();
                        report.cache_hits = cache.hits;
                        project(policy.report_path.as_deref(), &report);
                        last_project = Instant::now();
                    }
                }
            }
            let cursor = consumer.cursor();
            commit(
                &mut index,
                &ledger,
                Some(&mut consumer),
                &mut rows,
                cursor,
                true,
            )?;
            failpoint("publication");
            consumer.finish()?;
            ledger.source(&active_source, "committed", None)?;
        }
        // Specialized derivatives retain their original capture metadata and identity.
        for path in &policy.derivatives {
            active_source = path.display().to_string();
            ledger.source(&active_source, "pending", None)?;
            let fingerprint = crate::warc::consumer::checksum(File::open(path)?)?;
            let mut reader = BufReader::new(File::open(path)?);
            let mut ordinal = 0;
            loop {
                let mut line = Vec::new();
                let bytes = Read::by_ref(&mut reader)
                    .take(16 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut line)?;
                if bytes == 0 {
                    break;
                };
                anyhow::ensure!(
                    bytes <= 16 * 1024 * 1024,
                    "derived record exceeds byte budget"
                );
                ordinal += 1;
                let doc: super::derived::Document = serde_json::from_slice(&line)?;
                doc.source.validate()?;
                anyhow::ensure!(
                    doc.version == 1
                        && matches!(doc.kind.as_str(), "pdf-text-v1" | "offline-js-text-v1")
                        && doc.id == format!("{}:{}", doc.source.fetch_id, doc.kind),
                    "unknown derived version, kind or identity"
                );
                anyhow::ensure!(
                    doc.text.len() <= 2 * 1024 * 1024
                        && doc.title.len() <= 8192
                        && crate::capture::sha256(doc.text.as_bytes()) == doc.derived_sha256,
                    "derived digest/size mismatch"
                );
                let escape = |s: &str| {
                    s.replace('&', "&amp;")
                        .replace('<', "&lt;")
                        .replace('>', "&gt;")
                };
                let markup = format!(
                    "<html><head><title>{}</title></head><body><article>{}</article></body></html>",
                    escape(&doc.title),
                    escape(&doc.text)
                );
                let allowed = doc.indexing_allowed && !doc.source.robots.no_index;
                let mut meta = doc.source.clone();
                if !allowed {
                    meta.robots.no_index = true;
                }
                let record = WarcRecord {
                    request: crate::warc::Request {
                        url: doc.source.response.url.clone(),
                        date: Some(doc.source.response.received_at),
                    },
                    response: crate::warc::Response {
                        body: markup,
                        payload_type: Some(PayloadType::Html),
                    },
                    metadata: crate::warc::Metadata {
                        fetch_time_ms: doc.source.fetch_time_ms,
                        capture: Some(meta),
                    },
                };
                let mut row = decision(&record, &fingerprint, ordinal, &doc.kind)?;
                row.metadata = Some(doc.source.clone());
                if ledger.decided(&row.id)? {
                    continue;
                }
                ledger.plan(std::slice::from_ref(&row))?;
                let (mut row, document) = rt.block_on(ingest_record(
                    record,
                    row,
                    &mut index,
                    &ledger,
                    &mut HashMap::new(),
                    &worker,
                    &mut cache,
                    &mut std::collections::HashSet::new(),
                    Some(doc.text),
                    &base.warc_source,
                ))?;
                row.metadata = Some(doc.source);
                if !allowed {
                    row.reason = "derived_indexing_decision".into();
                }
                if let Some(document) = document {
                    anyhow::ensure!(
                        document.node_data.capacity() + 4096 <= policy.prepared_bytes,
                        "derived document exceeds prepared byte quota"
                    );
                    index.inverted_index.add_prepared_document(document)?;
                }
                commit(&mut index, &ledger, None, &mut vec![row], ordinal, true)?;
                report.source_records += 1;
            }
        }
        if !active_source.is_empty() {
            ledger.source(&active_source, "committed", None)?;
        }
        bounded_merge(&mut index, root, policy)?;
        Ok(())
    })();
    if let Err(error) = &result {
        let message = error.to_string();
        let cause = if CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
            "cancelled"
        } else if message.contains("deferred")
            || message.contains("quota")
            || message.contains("reserve")
        {
            "deferred"
        } else if message.contains("quarantined")
            || message.contains("corrupt")
            || message.contains("gzip")
            || message.contains("WARC")
            || message.contains("EOF")
        {
            "quarantined"
        } else {
            "storage_or_preparation_failed"
        };
        if !active_source.is_empty() {
            let _ = ledger.source(&active_source, cause, Some(&message));
        }
    }
    report.source_complete = result.is_ok();
    report.stage = if result.is_ok() {
        "committed"
    } else {
        "failed"
    }
    .into();
    report.error = result.as_ref().err().map(ToString::to_string);
    report.documents = ledger.documents()?;
    report.stored_versions = index.inverted_index.num_documents();
    report.outcomes = ledger.counts()?;
    report.recovered_reference_inputs = ledger
        .db
        .begin_read()?
        .open_table(COUNTS)?
        .get("reference_inputs")?
        .map_or(0, |v| v.value());
    report.stages = metrics::snapshot();
    report.cache_hits = cache.hits;
    report.elapsed_ms = started.elapsed().as_secs_f64() * 1000.;
    report.cpu_ms = (metrics::cpu_ns() - cpu) as f64 / 1e6;
    report.allocated_bytes =
        metrics::ALLOCATED.load(std::sync::atomic::Ordering::Relaxed) - allocated;
    project(policy.report_path.as_deref(), &report);
    result?;
    Ok(report)
}

pub fn preview(base: &IndexerConfig, query: &str) -> Result<serde_json::Value> {
    anyhow::ensure!(query.len() <= 4096, "query exceeds local preview budget");
    let root = Path::new(&base.output_path);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("ingestion.lock"))?;
    lock.try_lock_exclusive()
        .context("ingestion busy; retry preview")?;
    let ledger = Ledger::open(&root.join("ledger.redb"))?;
    let index = Index::open(root.join("working"))?;
    let mut seen = std::collections::HashSet::new();
    let mut results = Vec::new();
    let mut result_bytes = 0;
    for candidate in index.inverted_index.preview_documents(query)? {
        let (url, title, body) = candidate?;
        if let Some(current) = ledger.current(&url)? {
            if current.outcome == "indexed"
                && current.indexed_text_sha256.as_deref()
                    == Some(crate::capture::sha256(body.as_bytes()).as_str())
                && current
                    .indexed_document_sha256
                    .as_ref()
                    .is_none_or(|hash| *hash == document_identity(&url, &title, &body))
                && seen.insert(url.clone())
            {
                let result = serde_json::json!({"url":url,"title":title.chars().take(512).collect::<String>(),"snippet":body.chars().take(240).collect::<String>(),"version":current.version,"capture_id":current.capture_id,"identity_scope":if current.indexed_document_sha256.is_some(){"url-title-body-v1"}else{"legacy-body-v1"}});
                result_bytes += serde_json::to_vec(&result)?.len();
                if result_bytes > 192 * 1024 {
                    break;
                }
                results.push(result);
            }
        }
        if results.len() == 10 {
            break;
        }
    }
    Ok(
        serde_json::json!({"version":1,"consumer_contract":"ledger-current-v1","query":query,"results":results,"candidate_limit":1000}),
    )
}

fn document_identity(url: &str, title: &str, body: &str) -> String {
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    for field in [url, title, body] {
        digest.update(&(field.len() as u64).to_le_bytes());
        digest.update(field.as_bytes());
    }
    crate::capture::hex(digest.finish().as_ref())
}

/// Bounded offline inspection of business state; the dashboard uses projected counts.
pub fn audit(base: &IndexerConfig) -> Result<serde_json::Value> {
    let root = Path::new(&base.output_path);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join("ingestion.lock"))?;
    lock.try_lock_exclusive()
        .context("ingestion busy; retry audit")?;
    let ledger = Ledger::open(&root.join("ledger.redb"))?;
    let tx = ledger.db.begin_read()?;
    let mut rows: Vec<Decision> = Vec::new();
    for entry in tx.open_table(DECISIONS)?.iter()?.take(128) {
        let (_, value) = entry?;
        rows.push(serde_json::from_slice::<Decision>(value.value())?);
    }
    let mut current = Vec::new();
    for entry in tx.open_table(CURRENT)?.iter()?.take(128) {
        let (_, value) = entry?;
        current.push(serde_json::from_slice::<Decision>(value.value())?);
    }
    let mut sources = Vec::new();
    for entry in tx.open_table(SOURCES)?.iter()?.take(128) {
        let (id, value) = entry?;
        sources.push(serde_json::json!({"id":id.value(),"state":serde_json::from_slice::<serde_json::Value>(value.value())?}));
    }
    Ok(
        serde_json::json!({"version":1,"limit":128,"documents":ledger.documents()?,"outcomes":ledger.counts()?,"decisions":rows,"current":current,"sources":sources}),
    )
}
