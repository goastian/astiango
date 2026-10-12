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

use std::{future::Future, time::Duration};

use super::{
    pipeline::{Lease, Pipeline, PipelineConfig, PreparedPage},
    CrawlDatum, DatumSink, Error, Result,
};
use crate::{config::S3Config, warc};
use anyhow::anyhow;
use tokio::sync::{mpsc, oneshot};

type Acknowledgement = oneshot::Sender<std::result::Result<(), String>>;

/// Buffers WARC records and confirms remote persistence at each checkpoint.
pub struct WarcWriter {
    tx: mpsc::Sender<WarcWriterMessage>,
    pipeline: Pipeline,
}
impl WarcWriter {
    async fn acknowledged(
        &self,
        message: WarcWriterMessage,
        reply: oneshot::Receiver<std::result::Result<(), String>>,
    ) -> Result<()> {
        self.tx
            .send(message)
            .await
            .map_err(|_| anyhow!("WARC writer stopped"))?;
        reply
            .await
            .map_err(|_| anyhow!("WARC writer stopped before acknowledgement"))?
            .map_err(|message| Error::from(anyhow!(message)))
    }
    pub fn new(s3: S3Config) -> Self {
        Self::with_pipeline(
            s3,
            Pipeline::new(PipelineConfig::default()).expect("default pipeline"),
        )
    }
    pub(super) fn with_pipeline(s3: S3Config, pipeline: Pipeline) -> Self {
        let compressor = pipeline.clone();
        Self::with_pipeline_committer(pipeline, move |writer| {
            commit(writer, s3.clone(), compressor.clone())
        })
    }
    #[cfg(test)]
    fn with_committer<F, Fut>(commit: F) -> Self
    where
        F: Fn(warc::DeduplicatedWarcWriter) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        Self::with_pipeline_committer(Pipeline::new(PipelineConfig::default()).unwrap(), commit)
    }
    fn with_pipeline_committer<F, Fut>(pipeline: Pipeline, commit: F) -> Self
    where
        F: Fn(warc::DeduplicatedWarcWriter) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel(pipeline.stats().batch_queue.max_items + 1);
        tokio::spawn(writer_task(rx, commit, pipeline.clone()));
        Self { tx, pipeline }
    }
}
fn record_bytes(datum: &CrawlDatum) -> usize {
    datum
        .body
        .len()
        .saturating_add(datum.url.as_str().len().saturating_mul(2))
        .saturating_add(512)
}
impl DatumSink for WarcWriter {
    async fn write(&self, datum: CrawlDatum) -> Result<()> {
        self.write_prepared(PreparedPage::unleased(datum, None))
            .await
    }
    async fn write_prepared(&self, page: PreparedPage) -> Result<()> {
        let lease = self
            .pipeline
            .batch_queue
            .acquire(record_bytes(&page.datum))
            .await?;
        let (send, receive) = oneshot::channel();
        self.acknowledged(WarcWriterMessage::Crawl(page, lease, send), receive)
            .await
    }
    async fn checkpoint(&self) -> Result<()> {
        let (send, receive) = oneshot::channel();
        self.acknowledged(WarcWriterMessage::Checkpoint(send), receive)
            .await
    }
    async fn finish(&self) -> Result<()> {
        let (send, receive) = oneshot::channel();
        self.acknowledged(WarcWriterMessage::Finish(send), receive)
            .await
    }
}

enum WarcWriterMessage {
    Crawl(PreparedPage, Lease, Acknowledgement),
    Checkpoint(Acknowledgement),
    Finish(Acknowledgement),
}

async fn commit(
    writer: warc::DeduplicatedWarcWriter,
    s3: S3Config,
    pipeline: Pipeline,
) -> Result<()> {
    let filename = format!(
        "{}_{}.warc.gz",
        chrono::Utc::now().to_rfc3339(),
        uuid::Uuid::new_v4()
    );
    let data = pipeline
        .storage
        .run_durable(pipeline.stats().batch_segment_bytes, move || {
            writer.finish()
        })
        .await??;
    let bucket = s3::Bucket::new(
        &s3.bucket,
        s3::Region::Custom {
            region: String::new(),
            endpoint: s3.endpoint,
        },
        s3::creds::Credentials {
            access_key: Some(s3.access_key),
            secret_key: Some(s3.secret_key),
            security_token: None,
            session_token: None,
            expiration: None,
        },
    )
    .map_err(|error| anyhow!(error))?
    .with_path_style()
    .with_request_timeout(Duration::from_secs(30 * 60))
    .map_err(|error| anyhow!(error))?;
    let response = bucket
        .put_object_with_content_type(
            &format!("{}/{}", s3.folder, filename),
            &data,
            "application/warc",
        )
        .await
        .map_err(|error| anyhow!(error))?;
    if !(200..300).contains(&response.status_code()) {
        return Err(anyhow!("WARC upload returned HTTP {}", response.status_code()).into());
    }
    Ok(())
}

async fn writer_task<F, Fut>(
    mut rx: mpsc::Receiver<WarcWriterMessage>,
    commit: F,
    pipeline: Pipeline,
) where
    F: Fn(warc::DeduplicatedWarcWriter) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut writer = Some(warc::DeduplicatedWarcWriter::new());
    let mut input_bytes = 0usize;
    let mut input_items = 0usize;
    let limits = pipeline.stats();
    let mut failure: Option<String> = None;
    while let Some(message) = rx.recv().await {
        let (reply, flush, finished) = match message {
            WarcWriterMessage::Crawl(mut page, mut lease, reply) => {
                lease.start();
                let bytes = record_bytes(&page.datum);
                if failure.is_none() && bytes > limits.batch_segment_bytes {
                    failure = Some("WARC record exceeds pipeline batch segment byte limit".into());
                }
                if failure.is_none()
                    && input_items > 0
                    && (input_bytes.saturating_add(bytes) > limits.batch_segment_bytes
                        || input_items >= limits.batch_segment_items)
                {
                    if let Err(error) = commit(writer.take().unwrap()).await {
                        failure = Some(error.to_string());
                    }
                    writer = Some(warc::DeduplicatedWarcWriter::new());
                    input_bytes = 0;
                    input_items = 0;
                }
                if failure.is_none() {
                    let mut pending = writer.take().unwrap();
                    match pipeline
                        .storage
                        .run_durable(bytes, move || {
                            let record = warc::WarcRecord {
                                request: warc::Request {
                                    url: page.datum.url.to_string(),
                                    date: Some(page.datum.date),
                                },
                                response: warc::Response {
                                    body: std::mem::take(&mut page.datum.body),
                                    payload_type: Some(page.datum.payload_type),
                                },
                                metadata: warc::Metadata {
                                    fetch_time_ms: page.datum.fetch_time_ms,
                                },
                            };
                            let result = pending.write(&record);
                            drop(record);
                            drop(page);
                            (pending, result)
                        })
                        .await
                    {
                        Ok((pending, result)) => {
                            writer = Some(pending);
                            if let Err(error) = result {
                                failure = Some(error.to_string());
                            }
                        }
                        Err(error) => failure = Some(error.to_string()),
                    }
                    input_bytes = input_bytes.saturating_add(bytes);
                    input_items += 1;
                }
                drop(lease);
                (reply, false, false)
            }
            WarcWriterMessage::Checkpoint(reply) => (reply, true, false),
            WarcWriterMessage::Finish(reply) => (reply, true, true),
        };
        if flush && failure.is_none() && input_items > 0 {
            if let Err(error) = commit(writer.take().unwrap()).await {
                failure = Some(error.to_string());
            }
            writer = Some(warc::DeduplicatedWarcWriter::new());
            input_bytes = 0;
            input_items = 0;
        }
        let _ = reply.send(failure.as_ref().map_or(Ok(()), |error| Err(error.clone())));
        if finished {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    fn datum() -> CrawlDatum {
        CrawlDatum {
            url: url::Url::parse("https://example.org/page").unwrap(),
            payload_type: warc::PayloadType::Html,
            body: "<html><body>confirmed</body></html>".into(),
            fetch_time_ms: 7,
            date: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn checkpoint_confirms_all_preceding_writes_without_closing_shared_sink() {
        let uploads = Arc::new(AtomicUsize::new(0));
        let count = uploads.clone();
        let writer = WarcWriter::with_committer(move |records| {
            let count = count.clone();
            async move {
                assert!(records.num_writes() > 0);
                assert!(!records.finish()?.is_empty());
                count.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
        });
        writer.write(datum()).await.unwrap();
        assert_eq!(uploads.load(Ordering::SeqCst), 0);
        writer.checkpoint().await.unwrap();
        assert_eq!(uploads.load(Ordering::SeqCst), 1);
        writer.write(datum()).await.unwrap();
        writer.finish().await.unwrap();
        assert_eq!(uploads.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn upload_failure_reaches_checkpoint_future_writes_and_finish() {
        let writer = WarcWriter::with_committer(|_| async {
            Err(anyhow!("controlled upload failure").into())
        });
        writer.write(datum()).await.unwrap();
        assert!(writer
            .checkpoint()
            .await
            .unwrap_err()
            .to_string()
            .contains("controlled upload failure"));
        assert!(writer.write(datum()).await.is_err());
        assert!(writer.finish().await.is_err());
    }

    #[tokio::test]
    async fn segment_limits_rotate_before_adding_the_next_record() {
        for by_bytes in [false, true] {
            let size = record_bytes(&datum());
            let pipeline = Pipeline::new(PipelineConfig {
                batch_segment_bytes: if by_bytes { size * 2 } else { 4096 },
                batch_segment_items: if by_bytes { 100 } else { 2 },
                ..Default::default()
            })
            .unwrap();
            let batches = Arc::new(std::sync::Mutex::new(Vec::new()));
            let output = batches.clone();
            let writer = WarcWriter::with_pipeline_committer(pipeline.clone(), move |records| {
                output.lock().unwrap().push(records.num_writes());
                async { Ok(()) }
            });
            for n in 0..5 {
                let mut page = datum();
                page.url = url::Url::parse(&format!("https://example.org/pag{n}")).unwrap();
                writer.write(page).await.unwrap();
            }
            writer.finish().await.unwrap();
            pipeline.shutdown().await;
            assert_eq!(*batches.lock().unwrap(), [2, 2, 1]);
            assert_eq!(pipeline.stats().batch_queue.bytes, 0);
        }
    }

    #[tokio::test]
    async fn a_slow_upload_bounds_queued_data_and_finish_can_drain_after_cancellation() {
        let size = record_bytes(&datum());
        let mut cfg = PipelineConfig {
            batch_segment_items: 1,
            ..Default::default()
        };
        cfg.storage.max_items = 2;
        cfg.storage.max_bytes = size * 2 + 16;
        let pipeline = Pipeline::new(cfg).unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let first = Arc::new(AtomicUsize::new(0));
        let writer = Arc::new(WarcWriter::with_pipeline_committer(pipeline.clone(), {
            let entered = entered.clone();
            let release = release.clone();
            move |_| {
                let entered = entered.clone();
                let release = release.clone();
                let first = first.clone();
                async move {
                    if first.fetch_add(1, Ordering::SeqCst) == 0 {
                        entered.notify_one();
                        release.acquire().await.unwrap().forget();
                    }
                    Ok(())
                }
            }
        }));
        writer.write(datum()).await.unwrap();
        let second = {
            let writer = writer.clone();
            tokio::spawn(async move { writer.write(datum()).await })
        };
        entered.notified().await;
        let third = {
            let writer = writer.clone();
            tokio::spawn(async move { writer.write(datum()).await })
        };
        while pipeline.stats().batch_queue.items < 2 {
            tokio::task::yield_now().await;
        }
        let fourth = {
            let writer = writer.clone();
            tokio::spawn(async move { writer.write(datum()).await })
        };
        while pipeline.stats().batch_queue.admission_waits == 0 {
            tokio::task::yield_now().await;
        }
        assert_eq!(pipeline.stats().batch_queue.submitted, 3);
        fourth.abort();
        let _ = fourth.await;
        second.abort();
        let _ = second.await;
        let finish = {
            let writer = writer.clone();
            tokio::spawn(async move { writer.finish().await })
        };
        release.add_permits(1);
        third.await.unwrap().unwrap();
        finish.await.unwrap().unwrap();
        pipeline.shutdown().await;
        assert_eq!(pipeline.stats().batch_queue.items, 0);
    }
}
