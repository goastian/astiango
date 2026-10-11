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

use super::{CrawlDatum, DatumSink, Error, Result};
use crate::{config::S3Config, warc};
use anyhow::anyhow;
use tokio::sync::{mpsc, oneshot};

type Acknowledgement = oneshot::Sender<std::result::Result<(), String>>;

/// Buffers WARC records and confirms remote persistence at each checkpoint.
pub struct WarcWriter {
    tx: mpsc::Sender<WarcWriterMessage>,
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
        Self::with_committer(move |writer| commit(writer, s3.clone()))
    }

    fn with_committer<F, Fut>(commit: F) -> Self
    where
        F: Fn(warc::DeduplicatedWarcWriter) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let (tx, rx) = mpsc::channel(1);
        tokio::spawn(writer_task(rx, commit));
        Self { tx }
    }
}

impl DatumSink for WarcWriter {
    async fn write(&self, datum: CrawlDatum) -> Result<()> {
        let (send, receive) = oneshot::channel();
        self.acknowledged(WarcWriterMessage::Crawl(datum, send), receive)
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
    Crawl(CrawlDatum, Acknowledgement),
    Checkpoint(Acknowledgement),
    Finish(Acknowledgement),
}

async fn commit(writer: warc::DeduplicatedWarcWriter, s3: S3Config) -> Result<()> {
    let filename = format!(
        "{}_{}.warc.gz",
        chrono::Utc::now().to_rfc3339(),
        uuid::Uuid::new_v4()
    );
    let data = writer.finish()?;
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

async fn writer_task<F, Fut>(mut rx: mpsc::Receiver<WarcWriterMessage>, commit: F)
where
    F: Fn(warc::DeduplicatedWarcWriter) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let mut writer = warc::DeduplicatedWarcWriter::new();
    let mut failure: Option<String> = None;
    while let Some(message) = rx.recv().await {
        let (reply, flush, finished) = match message {
            WarcWriterMessage::Crawl(datum, reply) => {
                if failure.is_none() {
                    if let Err(error) = writer.write(&warc::WarcRecord {
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
                    }) {
                        failure = Some(error.to_string());
                    }
                }
                (reply, writer.num_bytes() > 1_000_000_000, false)
            }
            WarcWriterMessage::Checkpoint(reply) => (reply, true, false),
            WarcWriterMessage::Finish(reply) => (reply, true, true),
        };
        if flush && failure.is_none() && writer.num_writes() > 0 {
            let pending = std::mem::replace(&mut writer, warc::DeduplicatedWarcWriter::new());
            if let Err(error) = commit(pending).await {
                failure = Some(error.to_string());
            }
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
}
