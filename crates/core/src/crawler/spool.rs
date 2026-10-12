use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use fs4::FileExt;
use ring::digest::{Context as Digest, SHA256};
use serde::{Deserialize, Serialize};

use super::CrawlDatum;
use crate::warc;

const JOURNAL_LINE_LIMIT: u64 = 1024 * 1024;
pub const UPLOAD_PART_BYTES: usize = 5 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpoolConfig {
    pub directory: PathBuf,
    pub segment_bytes: u64,
    pub segment_records: usize,
    pub rotate_seconds: u64,
    pub gzip_level: u32,
    pub max_spool_bytes: u64,
    pub max_segments: usize,
    pub commit_records: usize,
    pub commit_wait_ms: u64,
    pub upload_retry_seconds: u64,
    pub upload_timeout_seconds: u64,
    pub gc_enabled: bool,
    pub retention_seconds: u64,
}

impl Default for SpoolConfig {
    fn default() -> Self {
        Self {
            directory: "data/crawler-spool".into(),
            segment_bytes: 128 * 1024 * 1024,
            segment_records: 65_536,
            rotate_seconds: 300,
            gzip_level: 3,
            max_spool_bytes: 8 * 1024 * 1024 * 1024,
            max_segments: 256,
            commit_records: 16,
            commit_wait_ms: 10,
            upload_retry_seconds: 10,
            upload_timeout_seconds: 60,
            gc_enabled: false,
            retention_seconds: 86400,
        }
    }
}

impl SpoolConfig {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            !self.directory.as_os_str().is_empty(),
            "empty spool directory"
        );
        anyhow::ensure!(
            (65536..=256 * 1024 * 1024).contains(&self.segment_bytes),
            "segment_bytes must be 64 KiB..256 MiB"
        );
        anyhow::ensure!(
            (1..=65536).contains(&self.segment_records),
            "invalid segment record limit"
        );
        anyhow::ensure!(self.gzip_level <= 9, "gzip_level must be 0..9");
        anyhow::ensure!(
            self.max_spool_bytes >= self.segment_bytes * 2
                && (1..=4096).contains(&self.max_segments),
            "invalid spool quota"
        );
        anyhow::ensure!(
            (1..=256).contains(&self.commit_records) && self.commit_wait_ms <= 1000,
            "invalid group commit limits"
        );
        anyhow::ensure!(
            self.rotate_seconds > 0
                && self.upload_retry_seconds > 0
                && self.upload_timeout_seconds > 0,
            "invalid spool intervals"
        );
        anyhow::ensure!(
            self.retention_seconds <= i64::MAX as u64,
            "retention interval exceeds supported timestamp range"
        );
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct SpoolStats {
    pub capture_records: u64,
    pub captured_payload_bytes: u64,
    pub decoded_payload_bytes: u64,
    pub noindex_records: u64,
    pub nofollow_records: u64,
    pub records_durable: u64,
    pub records_recovered: u64,
    pub segments_total: u64,
    pub segments_open: u64,
    pub segments_pending: u64,
    pub segments_published: u64,
    pub spool_bytes: u64,
    pub max_spool_bytes: u64,
    pub max_segments: usize,
    pub segment_bytes: u64,
    pub gzip_level: u32,
    pub group_commits: u64,
    pub sync_calls: u64,
    pub append_calls: u64,
    pub rotations: u64,
    pub upload_attempts: u64,
    pub upload_errors: u64,
    pub upload_buffer_bytes: u64,
    pub upload_buffer_peak_bytes: u64,
    pub quota_rejections: u64,
    pub errors: u64,
    pub reference_pins: u64,
    pub gc_blocked: u64,
    pub gc_deleted: u64,
    pub last_error: Option<String>,
}

#[derive(Clone, Default)]
pub struct SpoolMetrics(Arc<Mutex<SpoolStats>>);
impl SpoolMetrics {
    pub fn stats(&self) -> SpoolStats {
        self.0.lock().unwrap().clone()
    }
    pub(crate) fn update(&self, f: impl FnOnce(&mut SpoolStats)) {
        f(&mut self.0.lock().unwrap());
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Publication {
    Open,
    Local,
    Pending,
    Published,
    Deleting,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Segment {
    pub schema_version: u32,
    #[serde(default)]
    pub capture_versions: Vec<u32>,
    pub id: String,
    pub file: String,
    pub bytes: u64,
    pub records: u64,
    pub sha256: Option<String>,
    pub state: Publication,
    pub created_at: i64,
    pub sealed_at: Option<i64>,
    pub target: Option<String>,
    pub upload_id: Option<String>,
    pub references: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Receipt {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub record_offset: Option<u64>,
    pub capture_id: String,
    pub segment: String,
    pub start: u64,
    pub end: u64,
    pub sha256: String,
}

#[derive(Serialize, Deserialize)]
struct Commit {
    version: u32,
    start: u64,
    end: u64,
    receipts: Vec<Receipt>,
    sha256: String,
}

struct Active {
    id: String,
    data: File,
    journal: File,
    bytes: u64,
    records: u64,
    digest: Digest,
    opened: Instant,
}

pub struct Spool {
    pub config: SpoolConfig,
    pub metrics: SpoolMetrics,
    segments: BTreeMap<String, Segment>,
    active: Option<Active>,
    target: Option<String>,
    poisoned: Option<String>,
    _lock: File,
    #[cfg(any(test, feature = "crawler-bench"))]
    pub fault: Option<(String, bool, i32)>,
}

pub fn sha256(bytes: &[u8]) -> String {
    hex(ring::digest::digest(&SHA256, bytes).as_ref())
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn checksum(mut reader: impl Read) -> Result<String> {
    let mut digest = Digest::new(&SHA256);
    let mut buffer = [0u8; 65536];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex(digest.finish().as_ref()))
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension("json.tmp");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary)?;
    serde_json::to_writer(&mut file, value)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    File::open(path.parent().context("metadata has no parent")?)?.sync_all()?;
    Ok(())
}

fn create_durable_directory(path: &Path) -> Result<()> {
    let mut missing = Vec::new();
    let mut parent = path;
    while !parent.exists() && !parent.as_os_str().is_empty() {
        missing.push(parent.to_path_buf());
        parent = parent.parent().context("spool directory has no parent")?;
    }
    fs::create_dir_all(path)?;
    for created in missing.into_iter().rev() {
        File::open(&created)?.sync_all()?;
        File::open(
            created
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new(".")),
        )?
        .sync_all()?;
    }
    Ok(())
}

impl Spool {
    pub fn open(
        config: SpoolConfig,
        target: Option<String>,
        metrics: SpoolMetrics,
    ) -> Result<Self> {
        config.validate()?;
        create_durable_directory(&config.directory.join("segments"))?;
        create_durable_directory(&config.directory.join("state"))?;
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(config.directory.join("spool.lock"))?;
        lock.try_lock_exclusive()
            .context("another writer owns this spool")?;
        File::open(&config.directory)?.sync_all()?;
        if let Some(parent) = config
            .directory
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
        {
            File::open(parent)?.sync_all()?;
        }
        let mut spool = Self {
            config,
            metrics,
            segments: BTreeMap::new(),
            active: None,
            target,
            poisoned: None,
            _lock: lock,
            #[cfg(any(test, feature = "crawler-bench"))]
            fault: None,
        };
        spool.metrics.update(|s| {
            s.max_spool_bytes = spool.config.max_spool_bytes;
            s.max_segments = spool.config.max_segments;
            s.segment_bytes = spool.config.segment_bytes;
            s.gzip_level = spool.config.gzip_level;
        });
        for entry in fs::read_dir(spool.config.directory.join("state"))? {
            let path = entry?.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            anyhow::ensure!(
                spool.segments.len() < spool.config.max_segments,
                "spool segment limit exceeded during recovery"
            );
            let segment: Segment =
                serde_json::from_reader(File::open(&path)?.take(JOURNAL_LINE_LIMIT))?;
            anyhow::ensure!(
                matches!(segment.schema_version, 1 | 2)
                    && segment.capture_versions.iter().all(|version| *version == 1)
                    && uuid::Uuid::parse_str(&segment.id).is_ok(),
                "invalid segment identity"
            );
            anyhow::ensure!(
                segment.file == format!("{}.warc.gz", segment.id),
                "invalid segment filename"
            );
            anyhow::ensure!(
                path.file_name().and_then(|v| v.to_str()) == Some(&format!("{}.json", segment.id)),
                "segment descriptor identity mismatch"
            );
            anyhow::ensure!(
                segment.target == spool.target,
                "spool publication target changed; use another directory"
            );
            spool.segments.insert(segment.id.clone(), segment);
        }
        let ids: Vec<_> = spool.segments.keys().cloned().collect();
        for id in ids {
            let mut segment = spool.segments[&id].clone();
            if segment.state == Publication::Deleting {
                anyhow::ensure!(
                    segment.references.is_empty(),
                    "deleting segment still referenced"
                );
                spool.delete_files(&segment)?;
                spool.segments.remove(&id);
                continue;
            }
            if segment.state == Publication::Open {
                let (bytes, records, _) = spool.recover(&segment)?;
                if records == 0 {
                    segment.state = Publication::Deleting;
                    segment.references.clear();
                    atomic_json(&spool.meta_path(&id), &segment)?;
                    spool.delete_files(&segment)?;
                    spool.segments.remove(&id);
                    continue;
                }
                segment.bytes = bytes;
                segment.records = records;
                segment.sha256 = Some(checksum(File::open(spool.data_path(&segment.file))?)?);
                segment.state = if spool.target.is_some() {
                    Publication::Pending
                } else {
                    Publication::Local
                };
                segment.sealed_at = Some(chrono::Utc::now().timestamp());
                atomic_json(&spool.meta_path(&id), &segment)?;
                spool.metrics.update(|s| s.records_recovered += records);
                spool.segments.insert(id, segment);
            } else {
                anyhow::ensure!(
                    fs::metadata(spool.data_path(&segment.file))?.len() == segment.bytes,
                    "sealed segment length changed"
                );
                anyhow::ensure!(
                    segment.sha256.as_deref()
                        == Some(&checksum(File::open(spool.data_path(&segment.file))?)?),
                    "sealed segment checksum mismatch"
                );
            }
        }
        // Files created before their descriptor became durable never received an ACK.
        for entry in fs::read_dir(spool.config.directory.join("segments"))? {
            let path = entry?.path();
            let name = path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            let id = name.split('.').next().unwrap_or_default();
            if uuid::Uuid::parse_str(id).is_ok() && !spool.segments.contains_key(id) {
                let data = spool.data_path(&format!("{id}.warc.gz"));
                let journal = spool.journal_path(id);
                for file in [data, journal] {
                    match fs::metadata(file) {
                        Ok(metadata) => {
                            anyhow::ensure!(metadata.len() == 0,
                            "nonempty orphan segment may contain confirmed data; refusing deletion")
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error.into()),
                    }
                }
                fs::remove_file(path)?;
            }
        }
        for entry in fs::read_dir(spool.config.directory.join("state"))? {
            let path = entry?.path();
            let name = path
                .file_name()
                .and_then(|p| p.to_str())
                .unwrap_or_default();
            if name.ends_with(".json.tmp")
                && uuid::Uuid::parse_str(name.split('.').next().unwrap_or_default()).is_ok()
            {
                fs::remove_file(path)?;
            }
        }
        spool.refresh()?;
        Ok(spool)
    }

    fn data_path(&self, file: &str) -> PathBuf {
        self.config.directory.join("segments").join(file)
    }
    fn journal_path(&self, id: &str) -> PathBuf {
        self.config
            .directory
            .join("segments")
            .join(format!("{id}.commit"))
    }
    fn meta_path(&self, id: &str) -> PathBuf {
        self.config
            .directory
            .join("state")
            .join(format!("{id}.json"))
    }
    pub fn segments(&self) -> Vec<Segment> {
        self.segments.values().cloned().collect()
    }
    pub fn path(&self, segment: &Segment) -> PathBuf {
        self.data_path(&segment.file)
    }

    pub(super) fn hit(&mut self, point: &str) -> Result<()> {
        #[cfg(any(test, feature = "crawler-bench"))]
        if self
            .fault
            .as_ref()
            .is_some_and(|(name, _, _)| name == point)
        {
            let (_, crash, errno) = self.fault.take().unwrap();
            #[cfg(feature = "crawler-bench")]
            if crash && errno == 0 {
                super::capture_frontier_benchmark::barrier(point);
            }
            if crash {
                std::process::exit(86);
            }
            return Err(std::io::Error::from_raw_os_error(errno).into());
        }
        let _ = point;
        Ok(())
    }

    fn refresh(&self) -> Result<()> {
        let mut bytes = 0u64;
        for folder in ["segments", "state"] {
            for file in fs::read_dir(self.config.directory.join(folder))? {
                bytes = bytes
                    .checked_add(file?.metadata()?.len())
                    .context("spool size overflow")?;
            }
        }
        self.metrics.update(|s| {
            s.spool_bytes = bytes;
            s.segments_total = self.segments.len() as u64;
            s.segments_open = self
                .segments
                .values()
                .filter(|v| v.state == Publication::Open)
                .count() as u64;
            s.segments_pending = self
                .segments
                .values()
                .filter(|v| v.state == Publication::Pending)
                .count() as u64;
            s.segments_published = self
                .segments
                .values()
                .filter(|v| v.state == Publication::Published)
                .count() as u64;
            s.records_durable = self.segments.values().map(|v| v.records).sum();
            s.reference_pins = self
                .segments
                .values()
                .map(|v| v.references.len() as u64)
                .sum();
        });
        Ok(())
    }

    fn create_segment(&mut self) -> Result<()> {
        anyhow::ensure!(
            self.segments.len() < self.config.max_segments,
            "spool segment quota full"
        );
        self.hit("create")?;
        let id = uuid::Uuid::new_v4().to_string();
        let file = format!("{id}.warc.gz");
        let data = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(self.data_path(&file))?;
        let journal = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(self.journal_path(&id))?;
        data.sync_all()?;
        journal.sync_all()?;
        File::open(self.config.directory.join("segments"))?.sync_all()?;
        let segment = Segment {
            schema_version: 2,
            capture_versions: vec![1],
            id: id.clone(),
            file,
            bytes: 0,
            records: 0,
            sha256: None,
            state: Publication::Open,
            created_at: chrono::Utc::now().timestamp(),
            sealed_at: None,
            target: self.target.clone(),
            upload_id: None,
            references: BTreeMap::from([("consumer:archive".into(), "consumer".into())]),
        };
        atomic_json(&self.meta_path(&id), &segment)?;
        self.segments.insert(id.clone(), segment);
        self.active = Some(Active {
            id,
            data,
            journal,
            bytes: 0,
            records: 0,
            digest: Digest::new(&SHA256),
            opened: Instant::now(),
        });
        self.metrics.update(|s| s.sync_calls += 5);
        Ok(())
    }

    pub fn append(&mut self, records: &[&CrawlDatum]) -> Result<Vec<Receipt>> {
        anyhow::ensure!(
            self.poisoned.is_none(),
            "spool stopped after storage failure: {:?}",
            self.poisoned
        );
        anyhow::ensure!(
            !records.is_empty() && records.len() <= self.config.commit_records,
            "invalid commit group"
        );
        let result = self.append_inner(records);
        if let Err(error) = &result {
            self.fail(error.to_string());
        }
        result
    }

    pub(super) fn fail(&mut self, message: String) {
        self.poisoned = Some(message.clone());
        self.metrics.update(|s| {
            s.errors += 1;
            s.last_error = Some(message);
        });
    }

    fn append_inner(&mut self, records: &[&CrawlDatum]) -> Result<Vec<Receipt>> {
        let mut all = Vec::with_capacity(records.len());
        let mut index = 0;
        while index < records.len() {
            let first = records[index];
            let payload_size = first
                .capture
                .as_ref()
                .map_or(first.body.len(), |c| c.payload.len());
            let metadata_size = first
                .capture
                .as_ref()
                .map(|c| serde_json::to_vec(&c.metadata).map(|v| v.len()))
                .transpose()?
                .unwrap_or(0);
            let bound = payload_size as u64
                + metadata_size as u64 * 2
                + payload_size as u64 / 100
                + first.url.as_str().len() as u64
                + 8192;
            anyhow::ensure!(
                bound <= self.config.segment_bytes,
                "record exceeds segment byte limit"
            );
            if self.active.as_ref().is_some_and(|a| {
                a.records > 0
                    && (a.bytes + bound > self.config.segment_bytes
                        || a.records >= self.config.segment_records as u64
                        || a.opened.elapsed() >= Duration::from_secs(self.config.rotate_seconds))
            }) {
                self.seal()?;
            }
            self.refresh()?;
            let stats = self.metrics.stats();
            if stats.spool_bytes.saturating_add(bound + 16384) > self.config.max_spool_bytes
                || self.active.is_none() && self.segments.len() >= self.config.max_segments
            {
                self.metrics.update(|s| s.quota_rejections += 1);
                anyhow::bail!(
                    "spool quota full; admission stopped without deleting referenced data"
                );
            }
            if self.active.is_none() {
                self.create_segment()?;
            }
            let active = self.active.as_ref().unwrap();
            let mut end = index;
            let mut estimated = active.bytes;
            while end < records.len()
                && active.records + ((end - index) as u64) < self.config.segment_records as u64
            {
                let item = records[end];
                let upper = item.body.len() as u64
                    + item.body.len() as u64 / 100
                    + item.url.as_str().len() as u64
                    + 8192;
                if estimated + upper > self.config.segment_bytes
                    || stats.spool_bytes + estimated - active.bytes + upper + 16384
                        > self.config.max_spool_bytes
                {
                    break;
                }
                estimated += upper;
                end += 1;
            }
            anyhow::ensure!(end > index, "commit group cannot fit spool limits");
            all.extend(self.commit(&records[index..end])?);
            index = end;
        }
        Ok(all)
    }

    fn commit(&mut self, records: &[&CrawlDatum]) -> Result<Vec<Receipt>> {
        self.hit("append_before")?;
        let active = self.active.as_mut().unwrap();
        let start = active.bytes;
        let mut receipts = Vec::with_capacity(records.len());
        {
            let mut writer = BufWriter::with_capacity(65536, &mut active.data);
            let mut offset = start;
            for datum in records {
                let mut output = DigestWriter {
                    writer: &mut writer,
                    digest: Digest::new(&SHA256),
                    segment_digest: &mut active.digest,
                    bytes: 0,
                };
                let response_offset = if let Some(capture) = &datum.capture {
                    Some(warc::capture::write(
                        &mut output,
                        capture,
                        datum.payload_type,
                        self.config.gzip_level,
                        offset == 0,
                    )?)
                } else {
                    warc::write_gzip_members(
                        &mut output,
                        &warc::RecordRef {
                            url: datum.url.as_str(),
                            date: Some(&datum.date),
                            body: &datum.body,
                            payload_type: Some(datum.payload_type),
                            fetch_time_ms: datum.fetch_time_ms,
                        },
                        self.config.gzip_level,
                        offset == 0,
                    )?;
                    None
                };
                receipts.push(Receipt {
                    capture_version: datum.capture.as_ref().map(|c| c.metadata.schema_version),
                    record_offset: response_offset.map(|relative| offset + relative),
                    capture_id: datum.capture.as_ref().map_or_else(
                        || uuid::Uuid::new_v4().to_string(),
                        |c| c.metadata.fetch_id.clone(),
                    ),
                    segment: format!("{}.warc.gz", active.id),
                    start: offset,
                    end: offset + output.bytes,
                    sha256: hex(output.digest.finish().as_ref()),
                });
                offset = receipts.last().unwrap().end;
            }
            writer.flush()?;
        }
        self.metrics.update(|s| s.append_calls += 1);
        self.hit("append_after")?;
        self.hit("data_sync_before")?;
        self.active.as_ref().unwrap().data.sync_data()?;
        self.metrics.update(|s| s.sync_calls += 1);
        self.hit("data_sync_after")?;
        let end = receipts.last().unwrap().end;
        let encoded = serde_json::to_vec(&receipts)?;
        let commit = Commit {
            version: 1,
            start,
            end,
            receipts: receipts.clone(),
            sha256: sha256(&encoded),
        };
        let mut line = serde_json::to_vec(&commit)?;
        anyhow::ensure!(
            line.len() as u64 <= JOURNAL_LINE_LIMIT,
            "commit metadata exceeds limit"
        );
        line.push(b'\n');
        self.hit("journal_append_before")?;
        self.active.as_mut().unwrap().journal.write_all(&line)?;
        self.hit("journal_sync_before")?;
        self.active.as_ref().unwrap().journal.sync_data()?;
        self.metrics.update(|s| s.sync_calls += 1);
        self.hit("journal_sync_after")?;
        let active = self.active.as_mut().unwrap();
        active.bytes = end;
        active.records += records.len() as u64;
        let segment = self.segments.get_mut(&active.id).unwrap();
        segment.bytes = end;
        segment.records = active.records;
        for datum in records {
            if let Some(capture) = &datum.capture {
                if !segment
                    .capture_versions
                    .contains(&capture.metadata.schema_version)
                {
                    segment
                        .capture_versions
                        .push(capture.metadata.schema_version);
                }
                self.metrics.update(|stats| {
                    stats.capture_records += 1;
                    stats.captured_payload_bytes += capture.payload.len() as u64;
                    stats.decoded_payload_bytes += datum.body.len() as u64;
                    stats.noindex_records += u64::from(capture.metadata.robots.no_index);
                    stats.nofollow_records += u64::from(capture.metadata.robots.no_follow);
                });
            }
        }
        self.metrics.update(|s| s.group_commits += 1);
        self.refresh()?;
        Ok(receipts)
    }

    fn recover(&self, segment: &Segment) -> Result<(u64, u64, u64)> {
        let mut data = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.data_path(&segment.file))?;
        let journal = OpenOptions::new()
            .read(true)
            .write(true)
            .open(self.journal_path(&segment.id))?;
        let mut reader = BufReader::new(&journal);
        let mut end = 0;
        let mut records = 0;
        let mut journal_end = 0;
        loop {
            let mut line = Vec::new();
            let read = reader
                .by_ref()
                .take(JOURNAL_LINE_LIMIT + 2)
                .read_until(b'\n', &mut line)?;
            if read == 0 {
                break;
            }
            anyhow::ensure!(
                line.len() as u64 <= JOURNAL_LINE_LIMIT + 1,
                "commit journal line exceeds limit"
            );
            if line.last() != Some(&b'\n') {
                break;
            }
            let commit: Commit =
                serde_json::from_slice(&line).context("corrupt complete commit journal entry")?;
            anyhow::ensure!(
                commit.version == 1
                    && commit.start == end
                    && !commit.receipts.is_empty()
                    && commit.receipts.len() <= 256
                    && commit.sha256 == sha256(&serde_json::to_vec(&commit.receipts)?),
                "invalid durable commit"
            );
            for receipt in &commit.receipts {
                anyhow::ensure!(
                    receipt.segment == segment.file && receipt.start == end && receipt.end > end,
                    "invalid committed range"
                );
                data.seek(SeekFrom::Start(end))?;
                anyhow::ensure!(
                    checksum((&mut data).take(receipt.end - end))? == receipt.sha256
                        && data.stream_position()? == receipt.end,
                    "confirmed WARC range is missing or corrupt"
                );
                end = receipt.end;
                records += 1;
            }
            anyhow::ensure!(commit.end == end, "commit end mismatch");
            journal_end += read as u64;
        }
        data.set_len(end)?;
        journal.set_len(journal_end)?;
        data.sync_data()?;
        journal.sync_data()?;
        Ok((end, records, journal_end))
    }

    pub fn seal(&mut self) -> Result<()> {
        anyhow::ensure!(self.poisoned.is_none(), "cannot seal failed spool");
        if self.active.is_none() {
            return Ok(());
        }
        self.hit("seal_before")?;
        let active = self.active.as_ref().unwrap();
        let mut segment = self.segments[&active.id].clone();
        segment.sha256 = Some(hex(active.digest.clone().finish().as_ref()));
        segment.state = if self.target.is_some() {
            Publication::Pending
        } else {
            Publication::Local
        };
        segment.sealed_at = Some(chrono::Utc::now().timestamp());
        atomic_json(&self.meta_path(&active.id), &segment)?;
        self.metrics.update(|s| {
            s.sync_calls += 2;
            s.rotations += 1;
        });
        self.hit("seal_after")?;
        self.segments.insert(segment.id.clone(), segment);
        self.active = None;
        self.refresh()?;
        Ok(())
    }

    pub fn seal_due(&mut self) -> Result<()> {
        if self
            .active
            .as_ref()
            .is_some_and(|a| a.opened.elapsed() >= Duration::from_secs(self.config.rotate_seconds))
        {
            self.seal()?;
        }
        Ok(())
    }

    pub fn pending(&self) -> Option<Segment> {
        self.segments
            .values()
            .find(|s| s.state == Publication::Pending)
            .cloned()
    }

    pub fn remember_upload(&mut self, id: &str, upload_id: Option<String>) -> Result<()> {
        let mut segment = self.segments.get(id).context("unknown segment")?.clone();
        anyhow::ensure!(
            segment.state == Publication::Pending,
            "segment is not pending"
        );
        segment.upload_id = upload_id;
        atomic_json(&self.meta_path(id), &segment)?;
        self.segments.insert(id.into(), segment);
        Ok(())
    }

    pub fn published(&mut self, id: &str) -> Result<()> {
        self.hit("publish_confirm_before")?;
        let mut segment = self.segments.get(id).context("unknown segment")?.clone();
        anyhow::ensure!(
            segment.state == Publication::Pending,
            "segment is not pending"
        );
        segment.state = Publication::Published;
        segment.upload_id = None;
        atomic_json(&self.meta_path(id), &segment)?;
        self.hit("publish_confirm_after")?;
        self.segments.insert(id.into(), segment);
        self.refresh()
    }

    pub fn reference(&mut self, id: &str, owner: &str, kind: &str, retained: bool) -> Result<()> {
        anyhow::ensure!(
            ["revisit", "304", "consumer", "outbox", "backup"].contains(&kind)
                && !owner.is_empty()
                && owner.len() <= 256,
            "invalid retention reference"
        );
        let mut segment = self.segments.get(id).context("unknown segment")?.clone();
        anyhow::ensure!(
            segment.state != Publication::Deleting,
            "segment deletion already committed"
        );
        let key = format!("{kind}:{owner}");
        if retained {
            segment.references.insert(key, kind.into());
        } else {
            segment.references.remove(&key);
        }
        anyhow::ensure!(
            segment.references.len() <= 4096,
            "too many segment references"
        );
        atomic_json(&self.meta_path(id), &segment)?;
        self.segments.insert(id.into(), segment);
        self.refresh()
    }

    pub fn gc(&mut self) -> Result<u64> {
        if !self.config.gc_enabled {
            return Ok(0);
        }
        let candidates: Vec<_> = self.segments.values().cloned().collect();
        let now = chrono::Utc::now().timestamp();
        let mut deleted = 0;
        for mut segment in candidates {
            if segment.state != Publication::Published
                || !segment.references.is_empty()
                || segment.upload_id.is_some()
                || segment
                    .sealed_at
                    .is_none_or(|at| now.saturating_sub(at) < self.config.retention_seconds as i64)
            {
                self.metrics.update(|s| s.gc_blocked += 1);
                continue;
            }
            segment.state = Publication::Deleting;
            atomic_json(&self.meta_path(&segment.id), &segment)?;
            self.segments.insert(segment.id.clone(), segment.clone());
            self.delete_files(&segment)?;
            self.segments.remove(&segment.id);
            deleted += 1;
        }
        self.metrics.update(|s| s.gc_deleted += deleted);
        self.refresh()?;
        Ok(deleted)
    }

    fn delete_files(&self, segment: &Segment) -> Result<()> {
        for path in [
            self.data_path(&segment.file),
            self.journal_path(&segment.id),
        ] {
            if let Err(error) = fs::remove_file(path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    return Err(error.into());
                }
            }
        }
        File::open(self.config.directory.join("segments"))?.sync_all()?;
        fs::remove_file(self.meta_path(&segment.id))?;
        File::open(self.config.directory.join("state"))?.sync_all()?;
        Ok(())
    }
}

struct DigestWriter<'a, W> {
    writer: &'a mut W,
    digest: Digest,
    segment_digest: &'a mut Digest,
    bytes: u64,
}
impl<W: Write> Write for DigestWriter<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let count = self.writer.write(buffer)?;
        self.digest.update(&buffer[..count]);
        self.segment_digest.update(&buffer[..count]);
        self.bytes += count as u64;
        Ok(count)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.writer.flush()
    }
}

#[cfg(test)]
#[path = "spool_tests.rs"]
mod tests;
