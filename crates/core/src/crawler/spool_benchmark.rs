use super::{
    spool::{self, Receipt, Spool, SpoolConfig, SpoolMetrics},
    warc_writer, CrawlDatum,
};
use crate::{config::S3Config, warc};
use anyhow::Result;
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Instant,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub action: String,
    pub mode: String,
    pub spool: SpoolConfig,
    #[serde(default)]
    pub corpus_files: Vec<PathBuf>,
    #[serde(default)]
    pub pages: usize,
    #[serde(default)]
    pub body_bytes: usize,
    #[serde(default)]
    pub fault_point: Option<String>,
    pub s3: Option<S3Config>,
}

fn cpu_ms() -> f64 {
    let mut time = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_PROCESS_CPUTIME_ID, &mut time) },
        0
    );
    time.tv_sec as f64 * 1000.0 + time.tv_nsec as f64 / 1_000_000.0
}
fn bodies(config: &Config) -> Result<Vec<String>> {
    if config.body_bytes > 0 {
        anyhow::ensure!(
            config.body_bytes <= 8 * 1024 * 1024,
            "fixture body limit exceeded"
        );
        let mut rng = 0x61737469616e676fu64;
        let mut bytes = Vec::with_capacity(config.body_bytes);
        for _ in 0..config.body_bytes {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            bytes.push(b'!' + (rng % 90) as u8);
        }
        return Ok(vec![String::from_utf8(bytes)?]);
    }
    anyhow::ensure!(!config.corpus_files.is_empty(), "missing retained corpus");
    config
        .corpus_files
        .iter()
        .map(|p| Ok(fs::read_to_string(p)?))
        .collect()
}
fn page(body: &str, index: usize) -> CrawlDatum {
    CrawlDatum {
        url: url::Url::parse(&format!(
            "https://origin-{:03}.spool.test/page/{index}",
            index % 128
        ))
        .unwrap(),
        body: body.into(),
        payload_type: warc::PayloadType::Html,
        fetch_time_ms: 7,
        date: chrono::DateTime::parse_from_rfc3339("2026-10-12T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc),
    }
}
fn ack(directory: &std::path::Path, receipt: &Receipt) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(directory.join("acknowledged.jsonl"))?;
    serde_json::to_writer(&mut file, receipt)?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    File::open(directory)?.sync_all()?;
    Ok(())
}

pub async fn run(config: Config) -> Result<Value> {
    config.spool.validate()?;
    anyhow::ensure!(config.pages <= 8192, "fixture page limit exceeded");
    if let Some(s3) = &config.s3 {
        let url = url::Url::parse(&s3.endpoint)?;
        anyhow::ensure!(
            url.scheme() == "http" && url.host_str() == Some("127.0.0.1"),
            "test S3 must use explicit IPv4 loopback"
        );
    }
    let metrics = SpoolMetrics::default();
    let target = config
        .s3
        .as_ref()
        .map(|s| spool::sha256(format!("{}\n{}\n{}", s.endpoint, s.bucket, s.folder).as_bytes()));
    let mut disk = Spool::open(config.spool.clone(), target, metrics.clone())?;
    if config.action == "recover" {
        return Ok(json!({"stats": metrics.stats(), "segments": disk.segments()}));
    }
    if config.action == "publish" {
        let s3 = config.s3.context("publish fixture requires S3")?;
        if let Some(point) = &config.fault_point {
            disk.fault = Some((point.clone(), true, 5));
        }
        let disk = Arc::new(Mutex::new(disk));
        let pipeline = super::pipeline::Pipeline::new(Default::default())?;
        loop {
            let segment = disk.lock().unwrap().pending();
            let Some(segment) = segment else { break };
            metrics.update(|s| s.upload_attempts += 1);
            if let Err(error) = warc_writer::upload(&disk, &s3, &pipeline, &metrics, &segment).await
            {
                metrics.update(|s| {
                    s.upload_errors += 1;
                    s.last_error = Some(error.to_string());
                });
                break;
            }
        }
        let segments = disk.lock().unwrap().segments();
        pipeline.shutdown().await;
        return Ok(json!({"stats": metrics.stats(), "segments": segments}));
    }
    let corpus = bodies(&config)?;
    let expected: Vec<_> = (0..config.pages).map(|index| json!({
        "url": page("", index).url.as_str(), "sha256": spool::sha256(corpus[index % corpus.len()].as_bytes()),
    })).collect();
    if config.action == "online" {
        drop(disk);
        let pipeline = super::pipeline::Pipeline::new(Default::default())?;
        let writer = Arc::new(
            warc_writer::WarcWriter::open(
                config.spool.clone(),
                config.s3.clone(),
                pipeline.clone(),
                None,
            )
            .await?,
        );
        use super::DatumSink;
        let mut jobs = tokio::task::JoinSet::new();
        for index in 0..config.pages {
            let writer = writer.clone();
            let datum = page(&corpus[index % corpus.len()], index);
            jobs.spawn(async move { writer.write(datum).await });
            if jobs.len() >= 4 {
                jobs.join_next().await.unwrap()??;
            }
        }
        while let Some(result) = jobs.join_next().await {
            result??;
        }
        writer.checkpoint().await?;
        tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
        writer.finish().await?;
        let stats = writer.metrics.stats();
        drop(writer);
        pipeline.shutdown().await;
        let restored = Spool::open(
            config.spool.clone(),
            config.s3.as_ref().map(|s| {
                spool::sha256(format!("{}\n{}\n{}", s.endpoint, s.bucket, s.folder).as_bytes())
            }),
            Default::default(),
        )?;
        return Ok(
            json!({"stats": stats, "segments": restored.segments(), "expected": expected, "pipeline": pipeline.stats()}),
        );
    }
    if config.action == "fault" {
        let first = page(&corpus[0], 0);
        let first_ack = disk.append(&[&first])?.remove(0);
        ack(&config.spool.directory, &first_ack)?;
        let point = config.fault_point.as_ref().context("missing crash point")?;
        let second = page(&corpus[0], 1);
        if point.starts_with("seal") {
            let receipt = disk.append(&[&second])?.remove(0);
            ack(&config.spool.directory, &receipt)?;
            disk.fault = Some((point.clone(), true, 5));
            disk.seal()?;
        } else {
            if point == "create" {
                disk.seal()?;
            }
            disk.fault = Some((point.clone(), true, 5));
            disk.append(&[&second])?;
        }
        anyhow::bail!("crash point was not reached");
    }
    anyhow::ensure!(
        config.action == "write" && config.pages > 0,
        "unknown storage fixture action"
    );
    let before_cpu = cpu_ms();
    let started = Instant::now();
    let mut compressed = 0u64;
    let mut sync_calls = 0u64;
    let mut files = 0u64;
    if config.mode == "legacy_page" {
        anyhow::ensure!(
            config.spool.gzip_level == 9,
            "legacy fixture preserves gzip level 9"
        );
        let directory = config.spool.directory.join("legacy");
        fs::create_dir_all(&directory)?;
        for index in 0..config.pages {
            let datum = page(&corpus[index % corpus.len()], index);
            let mut writer = warc::WarcWriter::new();
            writer.write(&warc::WarcRecord {
                request: warc::Request {
                    url: datum.url.to_string(),
                    date: Some(datum.date),
                },
                response: warc::Response {
                    body: datum.body,
                    payload_type: Some(datum.payload_type),
                },
                metadata: warc::Metadata {
                    fetch_time_ms: datum.fetch_time_ms,
                },
            })?;
            let bytes = writer.finish()?;
            compressed += bytes.len() as u64;
            let path = directory.join(format!("{index}.warc.gz"));
            let temporary = path.with_extension("tmp");
            let mut file = File::create(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, &path)?;
            File::open(&directory)?.sync_all()?;
            sync_calls += 2;
            files += 1;
        }
    } else {
        anyhow::ensure!(config.mode == "segments", "unknown storage fixture mode");
        for index in (0..config.pages).step_by(config.spool.commit_records) {
            let batch: Vec<_> = (index..(index + config.spool.commit_records).min(config.pages))
                .map(|n| page(&corpus[n % corpus.len()], n))
                .collect();
            let records: Vec<_> = batch.iter().collect();
            disk.append(&records)?;
        }
        disk.seal()?;
        let stats = metrics.stats();
        sync_calls = stats.sync_calls;
        files = stats.segments_total;
        compressed = disk.segments().iter().map(|s| s.bytes).sum();
    }
    let duration_ms = started.elapsed().as_secs_f64() * 1000.0;
    let cpu_ms = cpu_ms() - before_cpu;
    Ok(json!({
        "pages": config.pages, "pages_per_second": config.pages as f64 * 1000.0 / duration_ms,
        "duration_ms": duration_ms, "cpu_ms_per_page": cpu_ms / config.pages as f64,
        "compressed_bytes": compressed, "compressed_bytes_per_page": compressed as f64 / config.pages as f64,
        "sync_calls": sync_calls, "sync_calls_per_page": sync_calls as f64 / config.pages as f64,
        "files": files, "files_per_page": files as f64 / config.pages as f64,
        "stats": metrics.stats(), "segments": disk.segments(), "expected": expected,
    }))
}

use anyhow::Context;
