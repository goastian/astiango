//! Read-only, bounded export of the historical FileQueue. Never advances its pointer.
use anyhow::{Context, Result};
use postcard::experimental::max_size::MaxSize;
use std::{fs::File, io::{Read, Seek, SeekFrom, Write}, path::Path};

#[derive(MaxSize, serde::Deserialize)]
struct Header { body_size: usize }

pub fn export(directory: &Path, mut output: impl Write) -> Result<u64> {
    let mut input = File::open(directory.join("data"))?;
    let size = input.metadata()?.len();
    let pointer = match std::fs::read(directory.join("pointer")) {
        Ok(bytes) => {
            if bytes.is_empty() { 0 } else {
                anyhow::ensure!(bytes.len() == 8, "invalid legacy queue pointer");
                u64::from_le_bytes(bytes.try_into().unwrap())
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error.into()),
    };
    anyhow::ensure!(pointer <= size, "legacy queue pointer exceeds file size");
    input.seek(SeekFrom::Start(pointer))?;
    let mut offset = pointer;
    let mut count = 0;
    while offset < size {
        let mut bytes = vec![0; Header::POSTCARD_MAX_SIZE];
        input.read_exact(&mut bytes).context("truncated legacy queue header")?;
        let header: Header = postcard::from_bytes(&bytes).context("invalid legacy queue header")?;
        anyhow::ensure!(header.body_size <= 8 * 1024 * 1024, "legacy job exceeds 8 MiB migration budget");
        let next = offset.checked_add(bytes.len() as u64).and_then(|n| n.checked_add(header.body_size as u64)).context("legacy queue offset overflow")?;
        anyhow::ensure!(next <= size, "truncated legacy queue body");
        bytes.resize(header.body_size, 0);
        input.read_exact(&mut bytes)?;
        let (job, consumed): (super::Job, usize) = bincode::decode_from_slice(&bytes, bincode::config::standard().with_limit::<8388608>())?;
        anyhow::ensure!(consumed == bytes.len(), "legacy job has trailing data");
        anyhow::ensure!(!job.urls.is_empty() || job.wandering_urls == 0, "domain-only wandering job requires explicit seed URLs before migration; original queue preserved");
        for entry in job.urls {
            serde_json::to_writer(&mut output, &serde_json::json!({"url":crate::capture::url_key(&entry.url)?,"provenance":{"kind":"legacy_file_queue","offset":offset,"weight":entry.weight,"wandering_urls":job.wandering_urls}}))?;
            output.write_all(b"\n")?;
            count += 1;
        }
        offset = next;
    }
    output.flush()?;
    Ok(count)
}
