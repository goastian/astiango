// Stract is an open source web search engine.
// Copyright (C) 2023 Stract ApS
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use super::{
    pipeline::{Lease, Pipeline, PreparedPage},
    spool::{self, Receipt, Segment, Spool, SpoolConfig, SpoolMetrics, UPLOAD_PART_BYTES},
    CrawlDatum, DatumSink, Error, Result,
};
use crate::config::S3Config;
use anyhow::{anyhow, Context};
use std::{
    fs::File,
    io::Read,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::{mpsc, oneshot, watch, RwLock};

type Reply = oneshot::Sender<std::result::Result<(), String>>;
pub(super) type CommitHook =
    Arc<dyn Fn(&PreparedPage, &Receipt) -> anyhow::Result<()> + Send + Sync>;

pub struct WarcWriter {
    tx: mpsc::Sender<Message>,
    pipeline: Pipeline,
    pub(super) metrics: SpoolMetrics,
    gate: RwLock<()>,
    accepting: AtomicBool,
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    uploader: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}
enum Message {
    Crawl(PreparedPage, Lease, Reply),
    Flush(Reply, bool),
}

fn record_bytes(datum: &CrawlDatum) -> usize {
    datum
        .memory_bytes()
        .saturating_add(datum.url.as_str().len())
        .saturating_add(8192)
}
impl WarcWriter {
    pub(super) async fn open(
        config: SpoolConfig,
        s3: Option<S3Config>,
        pipeline: Pipeline,
        hook: Option<CommitHook>,
    ) -> Result<Self> {
        config.validate().map_err(Error::Anyhow)?;
        let metrics = SpoolMetrics::default();
        let target = s3.as_ref().map(|s| {
            spool::sha256(format!("{}\n{}\n{}", s.endpoint, s.bucket, s.folder).as_bytes())
        });
        let observed = metrics.clone();
        let settings = config.clone();
        let disk = pipeline
            .storage
            .run_durable(0, move || Spool::open(settings, target, observed))
            .await??;
        let shared = Arc::new(Mutex::new(disk));
        let (tx, rx) = mpsc::channel(pipeline.stats().batch_queue.max_items + 1);
        let (stop, stopped) = watch::channel(false);
        let uploader = if let Some(s3) = s3 {
            Some(tokio::spawn(publisher(
                shared.clone(),
                s3,
                pipeline.clone(),
                metrics.clone(),
                stopped,
                config.upload_retry_seconds,
                config.upload_timeout_seconds,
            )))
        } else {
            None
        };
        let task = tokio::spawn(writer_task(
            rx,
            shared,
            pipeline.clone(),
            hook,
            stop,
            config,
        ));
        Ok(Self {
            tx,
            pipeline,
            metrics,
            gate: RwLock::new(()),
            accepting: AtomicBool::new(true),
            task: tokio::sync::Mutex::new(Some(task)),
            uploader: tokio::sync::Mutex::new(uploader),
        })
    }
    async fn acknowledge(
        &self,
        message: Message,
        receiver: oneshot::Receiver<std::result::Result<(), String>>,
    ) -> Result<()> {
        self.tx
            .send(message)
            .await
            .map_err(|_| anyhow!("durable WARC writer stopped"))?;
        receiver
            .await
            .map_err(|_| anyhow!("WARC writer stopped before acknowledgement"))?
            .map_err(|error| Error::Anyhow(anyhow!(error)))
    }
}
impl DatumSink for WarcWriter {
    async fn write(&self, datum: CrawlDatum) -> Result<()> {
        self.write_prepared(PreparedPage::unleased(datum, None))
            .await
    }
    async fn write_prepared(&self, page: PreparedPage) -> Result<()> {
        let gate = self.gate.read().await;
        if !self.accepting.load(Ordering::Acquire) {
            return Err(anyhow!("WARC writer is closing").into());
        }
        let lease = self
            .pipeline
            .batch_queue
            .acquire(record_bytes(&page.datum))
            .await?;
        let (send, receive) = oneshot::channel();
        self.tx
            .send(Message::Crawl(page, lease, send))
            .await
            .map_err(|_| anyhow!("WARC writer stopped"))?;
        drop(gate);
        receive
            .await
            .map_err(|_| anyhow!("WARC writer stopped before durable ACK"))?
            .map_err(|error| Error::Anyhow(anyhow!(error)))
    }
    async fn checkpoint(&self) -> Result<()> {
        let _gate = self.gate.read().await;
        if !self.accepting.load(Ordering::Acquire) {
            return Err(anyhow!("WARC writer is closing").into());
        }
        let (send, receive) = oneshot::channel();
        self.acknowledge(Message::Flush(send, false), receive).await
    }
    async fn finish(&self) -> Result<()> {
        let _gate = self.gate.write().await;
        if !self.accepting.swap(false, Ordering::AcqRel) {
            return Err(anyhow!("WARC writer already closed").into());
        }
        let (send, receive) = oneshot::channel();
        let result = self.acknowledge(Message::Flush(send, true), receive).await;
        if let Some(task) = self.task.lock().await.take() {
            task.await.map_err(|error| anyhow!(error))?;
        }
        if let Some(task) = self.uploader.lock().await.take() {
            task.await.map_err(|error| anyhow!(error))?;
        }
        result
    }
}
async fn writer_task(
    mut rx: mpsc::Receiver<Message>,
    disk: Arc<Mutex<Spool>>,
    pipeline: Pipeline,
    hook: Option<CommitHook>,
    stop: watch::Sender<bool>,
    config: SpoolConfig,
) {
    let mut pending = None;
    let mut failure: Option<String> = None;
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        let message = if let Some(message) = pending.take() {
            Some(message)
        } else {
            tokio::select! {
                message = rx.recv() => message,
                _ = tick.tick(), if failure.is_none() => {
                    let shared = disk.clone();
                    match pipeline.storage.run_durable(0, move || {
                        let mut spool = shared.lock().unwrap();
                        spool.seal_due()?;
                        spool.gc()?;
                        Ok::<_, anyhow::Error>(())
                    }).await {
                        Ok(Ok(())) => {}
                        result => {
                            let error = format!("spool rotation failed: {result:?}");
                            disk.lock().unwrap().fail(error.clone());
                            failure = Some(error);
                            pipeline.stop_admission();
                        }
                    }
                    continue;
                }
            }
        };
        let Some(message) = message else { break };
        match message {
            Message::Crawl(page, mut lease, reply) => {
                lease.start();
                let mut bytes = page.datum.memory_bytes();
                let mut batch = vec![(page, lease, reply)];
                let deadline =
                    tokio::time::Instant::now() + Duration::from_millis(config.commit_wait_ms);
                while batch.len() < config.commit_records && failure.is_none() {
                    match tokio::time::timeout_at(deadline, rx.recv()).await {
                        Ok(Some(Message::Crawl(page, mut lease, reply)))
                            if bytes.saturating_add(page.datum.memory_bytes())
                                <= pipeline.stats().storage.max_bytes =>
                        {
                            lease.start();
                            bytes += page.datum.memory_bytes();
                            batch.push((page, lease, reply));
                        }
                        Ok(Some(message)) => {
                            pending = Some(message);
                            break;
                        }
                        _ => break,
                    }
                }
                if let Some(error) = &failure {
                    for (_, _, reply) in batch {
                        let _ = reply.send(Err(error.clone()));
                    }
                    continue;
                }
                let shared = disk.clone();
                let callback = hook.clone();
                match pipeline
                    .storage
                    .run_durable(bytes, move || {
                        let records: Vec<_> =
                            batch.iter().map(|(page, _, _)| &page.datum).collect();
                        let committed = shared.lock().unwrap().append(&records);
                        drop(records);
                        let mut failure = committed.as_ref().err().map(ToString::to_string);
                        for (index, (page, _lease, reply)) in batch.into_iter().enumerate() {
                            let result = match &committed {
                                Ok(receipts) => callback
                                    .as_ref()
                                    .map_or(Ok(()), |f| f(&page, &receipts[index]))
                                    .map_err(|e| e.to_string()),
                                Err(error) => Err(error.to_string()),
                            };
                            if let Err(error) = &result {
                                if committed.is_ok() {
                                    shared.lock().unwrap().fail(error.clone());
                                }
                                failure.get_or_insert(error.clone());
                            }
                            let _ = reply.send(result);
                        }
                        failure
                    })
                    .await
                {
                    Ok(None) => {}
                    Ok(Some(error)) => {
                        failure = Some(error);
                        pipeline.stop_admission();
                    }
                    Err(error) => {
                        failure = Some(error.to_string());
                        pipeline.stop_admission();
                    }
                }
            }
            Message::Flush(reply, finished) => {
                if finished && failure.is_none() {
                    let shared = disk.clone();
                    if let Err(error) = async {
                        pipeline
                            .storage
                            .run_durable(0, move || shared.lock().unwrap().seal())
                            .await??;
                        Ok::<_, anyhow::Error>(())
                    }
                    .await
                    {
                        disk.lock().unwrap().fail(error.to_string());
                        failure = Some(error.to_string());
                        pipeline.stop_admission();
                    }
                }
                let _ = reply.send(failure.as_ref().map_or(Ok(()), |error| Err(error.clone())));
                if finished {
                    break;
                }
            }
        }
    }
    if failure.is_none() {
        let shared = disk.clone();
        if let Err(error) = async {
            pipeline
                .storage
                .run_durable(0, move || shared.lock().unwrap().seal())
                .await??;
            Ok::<_, anyhow::Error>(())
        }
        .await
        {
            disk.lock().unwrap().metrics.update(|s| {
                s.errors += 1;
                s.last_error = Some(error.to_string());
            });
            pipeline.stop_admission();
        }
    }
    let _ = stop.send(true);
}
async fn publisher(
    disk: Arc<Mutex<Spool>>,
    s3: S3Config,
    pipeline: Pipeline,
    metrics: SpoolMetrics,
    mut stop: watch::Receiver<bool>,
    retry_seconds: u64,
    timeout_seconds: u64,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(retry_seconds));
    loop {
        if *stop.borrow() {
            break;
        }
        tokio::select! {
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break; } }
            _ = interval.tick() => {
                loop {
                    let shared = disk.clone();
                    let segment = match pipeline.storage.run_durable(0, move || shared.lock().unwrap().pending()).await {
                        Ok(segment) => segment,
                        Err(error) => {
                            metrics.update(|s| { s.upload_errors += 1; s.last_error = Some(error.to_string()); });
                            break;
                        }
                    };
                    let Some(segment) = segment else { break };
                    metrics.update(|s| s.upload_attempts += 1);
                    let result = tokio::select! {
                        _ = stop.changed() => { metrics.update(|s| s.upload_buffer_bytes = 0); return; }
                        result = tokio::time::timeout(Duration::from_secs(timeout_seconds), upload(&disk, &s3, &pipeline, &metrics, &segment)) => result.map_err(|_| anyhow!("S3 upload total timeout")).and_then(|r| r),
                    };
                    metrics.update(|s| s.upload_buffer_bytes = 0);
                    if let Err(error) = result {
                        if error.is::<ArchiveFailure>() {
                            disk.lock().unwrap().fail(error.to_string());
                            pipeline.stop_admission();
                        }
                        metrics.update(|s| { s.upload_errors += 1; s.last_error = Some(error.to_string()); });
                        break;
                    }
                }
            }
        }
    }
}
#[derive(Debug, thiserror::Error)]
#[error("local archive operation failed: {0}")]
struct ArchiveFailure(String);

async fn archive_io<T, F>(pipeline: &Pipeline, bytes: usize, work: F) -> anyhow::Result<T>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    pipeline
        .storage
        .run_durable(bytes, work)
        .await
        .map_err(|error| ArchiveFailure(error.to_string()))?
        .map_err(|error| ArchiveFailure(error.to_string()).into())
}
pub(super) async fn upload(
    disk: &Arc<Mutex<Spool>>,
    s3: &S3Config,
    pipeline: &Pipeline,
    metrics: &SpoolMetrics,
    segment: &Segment,
) -> anyhow::Result<()> {
    let shared = disk.clone();
    let selected = segment.clone();
    let (path, timeout) = archive_io(pipeline, 0, move || {
        let spool = shared.lock().unwrap();
        Ok::<_, anyhow::Error>((spool.path(&selected), spool.config.upload_timeout_seconds))
    })
    .await?;
    let expected = segment.sha256.clone().context("segment has no checksum")?;
    let mut file = archive_io(pipeline, 0, move || {
        let mut file = File::open(&path)?;
        let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
        let mut buffer = [0u8; 65536];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hash.update(&buffer[..read]);
        }
        let actual: String = hash
            .finish()
            .as_ref()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        anyhow::ensure!(
            actual == expected,
            "segment checksum mismatch before upload"
        );
        use std::io::Seek;
        file.rewind()?;
        Ok::<_, anyhow::Error>(file)
    })
    .await?;
    let mut bucket = s3::Bucket::new(
        &s3.bucket,
        s3::Region::Custom {
            region: String::new(),
            endpoint: s3.endpoint.clone(),
        },
        s3::creds::Credentials {
            access_key: Some(s3.access_key.clone()),
            secret_key: Some(s3.secret_key.clone()),
            security_token: None,
            session_token: None,
            expiration: None,
        },
    )?
    .with_path_style()
    .with_request_timeout(Duration::from_secs(timeout))?;
    bucket.add_header("x-amz-meta-sha256", segment.sha256.as_ref().unwrap());
    let key = format!("{}/{}", s3.folder.trim_matches('/'), segment.file)
        .trim_start_matches('/')
        .to_string();
    if let Ok((head, status)) = bucket.head_object(&key).await {
        if status == 200
            && head.content_length == Some(segment.bytes as i64)
            && head.metadata.as_ref().and_then(|m| m.get("sha256")) == segment.sha256.as_ref()
        {
            let shared = disk.clone();
            let id = segment.id.clone();
            archive_io(pipeline, 0, move || shared.lock().unwrap().published(&id)).await?;
            return Ok(());
        }
    }
    if let Some(id) = &segment.upload_id {
        match bucket.abort_upload(&key, id).await {
            Ok(()) | Err(s3::error::S3Error::HttpFailWithBody(404, _)) => {}
            Err(error) => return Err(error.into()),
        }
    }
    if segment.bytes <= UPLOAD_PART_BYTES as u64 {
        let data = archive_io(pipeline, UPLOAD_PART_BYTES, move || {
            let mut bytes = Vec::with_capacity(UPLOAD_PART_BYTES);
            file.take(UPLOAD_PART_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            anyhow::ensure!(
                bytes.len() <= UPLOAD_PART_BYTES,
                "small upload exceeded buffer"
            );
            Ok::<_, anyhow::Error>(bytes)
        })
        .await?;
        metrics.update(|s| {
            s.upload_buffer_bytes = data.len() as u64;
            s.upload_buffer_peak_bytes = s.upload_buffer_peak_bytes.max(s.upload_buffer_bytes);
        });
        let response = bucket
            .put_object_with_content_type(&key, &data, "application/warc")
            .await?;
        anyhow::ensure!(
            (200..300).contains(&response.status_code()),
            "S3 PUT returned {}",
            response.status_code()
        );
        #[cfg(any(test, feature = "crawler-bench"))]
        {
            let shared = disk.clone();
            archive_io(pipeline, 0, move || {
                shared.lock().unwrap().hit("upload_after_transfer")
            })
            .await?;
        }
    } else {
        let initiated = bucket
            .initiate_multipart_upload(&key, "application/warc")
            .await?;
        let upload_id = initiated.upload_id;
        let shared = disk.clone();
        let id = segment.id.clone();
        let remembered = upload_id.clone();
        archive_io(pipeline, 0, move || {
            shared
                .lock()
                .unwrap()
                .remember_upload(&id, Some(remembered))
        })
        .await?;
        let mut parts = Vec::new();
        let mut remaining = segment.bytes;
        while remaining > 0 {
            let result = archive_io(pipeline, UPLOAD_PART_BYTES, move || {
                let mut bytes = vec![0u8; UPLOAD_PART_BYTES.min(remaining as usize)];
                file.read_exact(&mut bytes)?;
                Ok::<_, anyhow::Error>((file, bytes))
            })
            .await?;
            file = result.0;
            let chunk = result.1;
            remaining -= chunk.len() as u64;
            metrics.update(|s| {
                s.upload_buffer_bytes = chunk.len() as u64;
                s.upload_buffer_peak_bytes = s.upload_buffer_peak_bytes.max(s.upload_buffer_bytes);
            });
            parts.push(
                bucket
                    .put_multipart_chunk(
                        chunk,
                        &key,
                        parts.len() as u32 + 1,
                        &upload_id,
                        "application/warc",
                    )
                    .await?,
            );
            #[cfg(any(test, feature = "crawler-bench"))]
            {
                let shared = disk.clone();
                archive_io(pipeline, 0, move || {
                    shared.lock().unwrap().hit("upload_after_part")
                })
                .await?;
            }
        }
        let response = bucket
            .complete_multipart_upload(&key, &upload_id, parts)
            .await?;
        anyhow::ensure!(
            (200..300).contains(&response.status_code())
                && response.as_str()?.contains("CompleteMultipartUploadResult")
                && !response.as_str()?.contains("<Error>"),
            "S3 multipart completion failed"
        );
        #[cfg(any(test, feature = "crawler-bench"))]
        {
            let shared = disk.clone();
            archive_io(pipeline, 0, move || {
                shared.lock().unwrap().hit("upload_after_transfer")
            })
            .await?;
        }
    }
    let (head, status) = bucket.head_object(&key).await?;
    anyhow::ensure!(
        status == 200
            && head.content_length == Some(segment.bytes as i64)
            && head
                .metadata
                .as_ref()
                .and_then(|m| m.get("sha256").or_else(|| m.get("x-amz-meta-sha256")))
                == segment.sha256.as_ref(),
        "S3 confirmation size/checksum mismatch"
    );
    let shared = disk.clone();
    let id = segment.id.clone();
    archive_io(pipeline, 0, move || shared.lock().unwrap().published(&id)).await?;
    metrics.update(|s| s.upload_buffer_bytes = 0);
    Ok(())
}
