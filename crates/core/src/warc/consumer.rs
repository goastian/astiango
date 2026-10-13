use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use fs4::FileExt;
use serde::{Deserialize, Serialize};

use super::{
    stream::{self, Limits},
    RecordIterator, WarcRecord,
};
use crate::{config::WarcSource, Result};

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint {
    version: u32,
    fingerprint: String,
    records_committed: u64,
    complete: bool,
    quarantine: Option<String>,
}

pub struct Batch {
    pub records: Vec<WarcRecord>,
    pub next_record: u64,
    pub bytes: usize,
}

pub struct Consumer {
    reader: RecordIterator<Box<dyn Read>>,
    state: Checkpoint,
    path: PathBuf,
    _lock: File,
    cursor: u64,
    pending: Option<WarcRecord>,
    limits: Limits,
}

#[derive(Serialize, Deserialize)]
struct Commit {
    consumer_version: u32,
    fingerprint: String,
    records_committed: u64,
    complete: bool,
    keys: Vec<String>,
}

impl Consumer {
    pub fn open(source: &WarcSource, path: &str, directory: &Path, limits: Limits) -> Result<Self> {
        limits.validate()?;
        fs::create_dir_all(directory)?;
        let identity = match source {
            WarcSource::Local(config) => format!("local:{}:{path}", config.folder),
            WarcSource::HTTP(config) => format!("http:{}:{path}", config.base_url),
            WarcSource::S3(config) => format!("s3:{}:{}:{path}", config.endpoint, config.bucket),
        };
        let key = crate::capture::sha256(identity.as_bytes());
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join(format!("{key}.lock")))?;
        lock.try_lock_exclusive()?;
        let state_path = directory.join(format!("{key}.json"));
        let local_path = match source {
            WarcSource::Local(config) => Path::new(&config.folder).join(path),
            _ => {
                let target = directory.join(format!("{key}.input.warc.gz"));
                let temporary = directory.join(format!("{key}.{}.download", uuid::Uuid::new_v4()));
                let mut output = OpenOptions::new()
                    .create_new(true)
                    .write(true)
                    .open(&temporary)?;
                let download: Result<()> = (|| {
                    let mut reader =
                        stream::open_reader(source, path)?.take(4 * 1024 * 1024 * 1024 + 1);
                    let bytes = std::io::copy(&mut reader, &mut output)?;
                    anyhow::ensure!(
                        bytes <= 4 * 1024 * 1024 * 1024,
                        "consumer input exceeds 4 GiB staging quota"
                    );
                    output.sync_all()?;
                    if state_path.exists() {
                        let previous: Checkpoint =
                            serde_json::from_reader(File::open(&state_path)?.take(65536))?;
                        anyhow::ensure!(
                            previous.version == 1,
                            "unsupported consumer checkpoint version"
                        );
                        anyhow::ensure!(
                            previous.fingerprint == checksum(File::open(&temporary)?)?,
                            "remote WARC changed since checkpoint; previous staged input preserved"
                        );
                    }
                    fs::rename(&temporary, &target)?;
                    File::open(directory)?.sync_all()?;
                    Ok(())
                })();
                if download.is_err() {
                    let _ = fs::remove_file(temporary);
                }
                download?;
                target
            }
        };
        let fingerprint = checksum(File::open(&local_path)?)?;
        let state: Checkpoint = if state_path.exists() {
            let state: Checkpoint = serde_json::from_reader(File::open(&state_path)?.take(65536))?;
            anyhow::ensure!(
                state.version == 1,
                "unsupported consumer checkpoint version {}",
                state.version
            );
            anyhow::ensure!(
                state.fingerprint == fingerprint,
                "WARC changed since consumer checkpoint; quarantined input must be reviewed"
            );
            anyhow::ensure!(
                state.quarantine.is_none(),
                "WARC input is quarantined: {:?}",
                state.quarantine
            );
            state
        } else {
            Checkpoint {
                version: 1,
                fingerprint,
                ..Default::default()
            }
        };
        let reader = RecordIterator::with_limits(
            Box::new(File::open(&local_path)?) as Box<dyn Read>,
            limits,
        );
        let mut consumer = Self {
            reader,
            state,
            path: state_path,
            _lock: lock,
            cursor: 0,
            pending: None,
            limits,
        };
        for _ in 0..consumer.state.records_committed {
            match consumer.reader.next() {
                Some(Ok(_)) => consumer.cursor += 1,
                Some(Err(error)) => return consumer.fail(error),
                None => {
                    return consumer.fail(anyhow::anyhow!("checkpoint extends beyond the input"))
                }
            }
        }
        Ok(consumer)
    }

    pub fn next_batch(&mut self, items: usize) -> Result<Option<Batch>> {
        anyhow::ensure!(items > 0, "consumer batch size must be positive");
        if self.state.complete {
            return Ok(None);
        }
        let mut records = Vec::new();
        let mut bytes = 0usize;
        while records.len() < items {
            let record = if let Some(record) = self.pending.take() {
                Some(Ok(record))
            } else {
                self.reader.next()
            };
            let record = match record {
                Some(Ok(record)) => record,
                Some(Err(error)) => return self.fail(error),
                None => break,
            };
            let size = record
                .response
                .body
                .capacity()
                .saturating_add(record.request.url.capacity())
                .saturating_add(
                    record
                        .metadata
                        .capture
                        .as_ref()
                        .map_or(0, crate::capture::Metadata::memory_bytes),
                )
                .saturating_add(std::mem::size_of::<WarcRecord>());
            if size > self.limits.batch_bytes {
                return self.fail(anyhow::anyhow!("record exceeds consumer batch byte budget"));
            }
            if !records.is_empty() && bytes.saturating_add(size) > self.limits.batch_bytes {
                self.pending = Some(record);
                break;
            }
            bytes += size;
            records.push(record);
            self.cursor += 1;
        }
        if records.is_empty() {
            return Ok(None);
        }
        Ok(Some(Batch {
            records,
            next_record: self.cursor,
            bytes,
        }))
    }

    pub fn recover_commit(&mut self, payload: Option<&str>) -> Result<Vec<String>> {
        let Some(payload) = payload else {
            return Ok(Vec::new());
        };
        let Ok(commit) = serde_json::from_str::<Commit>(payload) else {
            return Ok(Vec::new());
        };
        anyhow::ensure!(
            commit.consumer_version == 1,
            "unsupported consumer commit version"
        );
        if commit.fingerprint != self.state.fingerprint {
            return Ok(Vec::new());
        }
        while self.cursor < commit.records_committed {
            match self.reader.next() {
                Some(Ok(_)) => self.cursor += 1,
                Some(Err(error)) => return self.fail(error),
                None => {
                    return self.fail(anyhow::anyhow!("durable engine cursor extends beyond WARC"))
                }
            }
        }
        self.confirm(commit.records_committed, commit.complete)?;
        Ok(commit.keys)
    }

    pub fn payload(&self, next_record: u64, complete: bool, keys: Vec<String>) -> Result<String> {
        Ok(serde_json::to_string(&Commit {
            consumer_version: 1,
            fingerprint: self.state.fingerprint.clone(),
            records_committed: next_record,
            complete,
            keys,
        })?)
    }

    pub fn cursor(&self) -> u64 {
        self.cursor
    }

    pub fn confirm(&mut self, next_record: u64, complete: bool) -> Result<()> {
        anyhow::ensure!(
            next_record >= self.state.records_committed && next_record <= self.cursor,
            "invalid consumer confirmation"
        );
        self.state.records_committed = next_record;
        self.state.complete = complete;
        atomic_json(&self.path, &self.state)
    }

    pub fn finish(&mut self) -> Result<()> {
        self.confirm(self.cursor, true)
    }

    fn fail<T>(&mut self, error: anyhow::Error) -> Result<T> {
        self.state.quarantine = Some(error.to_string().chars().take(2048).collect());
        atomic_json(&self.path, &self.state)?;
        Err(error.context(format!("WARC quarantined at {}", self.path.display())))
    }
}

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temporary)?;
    let result = (|| {
        serde_json::to_writer(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        File::open(path.parent().unwrap_or(Path::new(".")))?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}

fn checksum(mut reader: impl Read) -> Result<String> {
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = [0u8; 65536];
    loop {
        let size = reader.read(&mut buffer)?;
        if size == 0 {
            break;
        }
        digest.update(&buffer[..size]);
    }
    Ok(crate::capture::hex(digest.finish().as_ref()))
}
