use super::*;
use crate::crawler::{pipeline::Pipeline, warc_writer::WarcWriter, DatumSink};

fn config(path: &Path) -> SpoolConfig {
    SpoolConfig {
        directory: path.into(),
        segment_bytes: 65536,
        segment_records: 8,
        max_spool_bytes: 262144,
        max_segments: 8,
        commit_records: 8,
        commit_wait_ms: 20,
        retention_seconds: 0,
        ..Default::default()
    }
}
fn datum(index: usize) -> CrawlDatum {
    CrawlDatum {
        url: url::Url::parse(&format!("https://example.org/{index}")).unwrap(),
        body: format!(
            "<html><body>日本語 español {index} {}</body></html>",
            "bounded content ".repeat(100)
        ),
        payload_type: warc::PayloadType::Html,
        fetch_time_ms: 7,
        date: chrono::Utc::now(),
    }
}
fn load(spool: &Spool) -> Vec<warc::WarcRecord> {
    spool
        .segments()
        .iter()
        .flat_map(|segment| {
            warc::WarcFile::open(spool.path(segment))
                .unwrap()
                .records()
                .collect::<Vec<_>>()
        })
        .map(Result::unwrap)
        .collect()
}

#[test]
fn streaming_members_preserve_existing_reader_fields_and_multiple_pages() {
    let dir = tempfile::tempdir().unwrap();
    let mut spool = Spool::open(config(dir.path()), None, Default::default()).unwrap();
    let first = datum(1);
    let second = datum(2);
    let receipts = spool.append(&[&first, &second]).unwrap();
    assert_eq!(receipts.len(), 2);
    assert_eq!(receipts[0].start, 0);
    assert_eq!(receipts[0].end, receipts[1].start);
    spool.seal().unwrap();
    let records = load(&spool);
    for (record, expected) in records.iter().zip([first, second]) {
        assert_eq!(record.request.url, expected.url.as_str());
        assert_eq!(record.request.date, Some(expected.date));
        assert_eq!(record.response.body, expected.body);
        assert_eq!(record.response.payload_type, Some(expected.payload_type));
        assert_eq!(record.metadata.fetch_time_ms, expected.fetch_time_ms);
    }
    let stats = spool.metrics.stats();
    assert_eq!(stats.records_durable, 2);
    assert_eq!(stats.group_commits, 1);
    assert_eq!(stats.segments_total, 1);
}

#[test]
fn recovery_keeps_confirmed_ranges_and_truncates_only_uncommitted_tail() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let mut spool = Spool::open(cfg.clone(), None, Default::default()).unwrap();
    let receipt = spool.append(&[&datum(1)]).unwrap().remove(0);
    let segment = spool.segments()[0].clone();
    OpenOptions::new()
        .append(true)
        .open(spool.path(&segment))
        .unwrap()
        .write_all(b"unfinished compressed tail")
        .unwrap();
    OpenOptions::new()
        .append(true)
        .open(spool.journal_path(&segment.id))
        .unwrap()
        .write_all(b"{\"version\":")
        .unwrap();
    drop(spool);
    let restored = Spool::open(cfg, None, Default::default()).unwrap();
    assert_eq!(restored.metrics.stats().records_recovered, 1);
    assert_eq!(
        fs::metadata(restored.path(&segment)).unwrap().len(),
        receipt.end
    );
    assert_eq!(load(&restored)[0].request.url, datum(1).url.as_str());
}

#[test]
fn confirmed_corruption_is_reported_and_never_silently_truncated() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let mut spool = Spool::open(cfg.clone(), None, Default::default()).unwrap();
    spool.append(&[&datum(1)]).unwrap();
    let segment = spool.segments()[0].clone();
    let path = spool.path(&segment);
    let size = fs::metadata(&path).unwrap().len();
    drop(spool);
    let mut file = OpenOptions::new().write(true).open(&path).unwrap();
    file.seek(SeekFrom::Start(20)).unwrap();
    file.write_all(b"corrupt").unwrap();
    let error = Spool::open(cfg, None, Default::default()).err().unwrap();
    assert!(error.to_string().contains("corrupt"));
    assert_eq!(fs::metadata(&path).unwrap().len(), size);
}

#[test]
fn missing_descriptor_never_causes_confirmed_archive_deletion() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let mut spool = Spool::open(cfg.clone(), None, Default::default()).unwrap();
    spool.append(&[&datum(1)]).unwrap();
    let segment = spool.segments()[0].clone();
    let path = spool.path(&segment);
    fs::remove_file(spool.meta_path(&segment.id)).unwrap();
    drop(spool);
    assert!(Spool::open(cfg, None, Default::default())
        .err()
        .unwrap()
        .to_string()
        .contains("orphan"));
    assert!(path.exists());
    assert_eq!(warc::WarcFile::open(path).unwrap().records().count(), 1);
}

#[test]
fn sealed_checksum_is_checked_during_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let mut spool = Spool::open(cfg.clone(), None, Default::default()).unwrap();
    spool.append(&[&datum(1)]).unwrap();
    spool.seal().unwrap();
    let path = spool.path(&spool.segments()[0]);
    drop(spool);
    let mut file = OpenOptions::new().write(true).open(path).unwrap();
    file.seek(SeekFrom::Start(20)).unwrap();
    file.write_all(b"corrupt").unwrap();
    assert!(Spool::open(cfg, None, Default::default())
        .err()
        .unwrap()
        .to_string()
        .contains("checksum"));
}

#[test]
fn append_and_sync_failures_preserve_previous_ack_and_stop_the_writer() {
    for point in [
        "append_before",
        "append_after",
        "data_sync_before",
        "data_sync_after",
        "journal_append_before",
        "journal_sync_before",
        "journal_sync_after",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        let mut spool = Spool::open(cfg.clone(), None, Default::default()).unwrap();
        spool.append(&[&datum(1)]).unwrap();
        spool.fault = Some((point.into(), false, 5));
        assert!(spool.append(&[&datum(2)]).is_err(), "{point}");
        assert!(spool.append(&[&datum(3)]).is_err());
        drop(spool);
        let restored = Spool::open(cfg, None, Default::default()).unwrap();
        assert!(
            load(&restored)
                .iter()
                .any(|record| record.request.url == datum(1).url.as_str()),
            "{point}"
        );
    }
}

#[test]
fn closure_failures_leave_every_ack_recoverable() {
    for point in ["seal_before", "seal_after"] {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        let mut spool = Spool::open(cfg.clone(), None, Default::default()).unwrap();
        spool.append(&[&datum(1), &datum(2)]).unwrap();
        spool.fault = Some((point.into(), false, 5));
        assert!(spool.seal().is_err());
        drop(spool);
        let restored = Spool::open(cfg, None, Default::default()).unwrap();
        assert_eq!(load(&restored).len(), 2);
    }
}

#[test]
fn quota_stops_without_collecting_referenced_or_pending_segments() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.max_segments = 1;
    let mut spool = Spool::open(cfg.clone(), Some("fixture".into()), Default::default()).unwrap();
    spool.append(&[&datum(1)]).unwrap();
    spool.seal().unwrap();
    let error = spool.append(&[&datum(2)]).unwrap_err();
    assert!(error.to_string().contains("quota"));
    assert_eq!(spool.metrics.stats().quota_rejections, 1);
    assert_eq!(load(&spool).len(), 1);
    drop(spool);
    let recovered = Spool::open(cfg, Some("fixture".into()), Default::default()).unwrap();
    assert_eq!(recovered.segments()[0].state, Publication::Pending);
}

#[test]
fn byte_quota_stops_before_exceeding_disk_budget_and_retains_all_acks() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.max_spool_bytes = 131072;
    cfg.gzip_level = 0;
    let mut spool = Spool::open(cfg.clone(), None, Default::default()).unwrap();
    let mut accepted = 0;
    for index in 0..100 {
        let mut page = datum(index);
        page.body = "x".repeat(16384);
        match spool.append(&[&page]) {
            Ok(_) => accepted += 1,
            Err(error) => {
                assert!(error.to_string().contains("quota"));
                break;
            }
        }
    }
    assert!(accepted > 0 && accepted < 100);
    assert!(spool.segments().len() < cfg.max_segments);
    assert!(spool.metrics.stats().spool_bytes <= cfg.max_spool_bytes);
    drop(spool);
    assert_eq!(
        load(&Spool::open(cfg, None, Default::default()).unwrap()).len(),
        accepted
    );
}

#[test]
fn inode_exhaustion_at_segment_creation_retains_sealed_data() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let mut spool = Spool::open(cfg.clone(), None, Default::default()).unwrap();
    spool.append(&[&datum(1)]).unwrap();
    spool.seal().unwrap();
    spool.fault = Some(("create".into(), false, 28));
    let error = spool.append(&[&datum(2)]).unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .raw_os_error(),
        Some(28)
    );
    drop(spool);
    assert_eq!(
        load(&Spool::open(cfg, None, Default::default()).unwrap()).len(),
        1
    );
}

#[cfg(target_os = "linux")]
#[test]
fn kernel_enospc_does_not_remove_confirmed_data() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let mut spool = Spool::open(cfg.clone(), None, Default::default()).unwrap();
    spool.append(&[&datum(1)]).unwrap();
    spool.active.as_mut().unwrap().data = OpenOptions::new().write(true).open("/dev/full").unwrap();
    let error = spool.append(&[&datum(2)]).unwrap_err();
    assert_eq!(
        error
            .downcast_ref::<std::io::Error>()
            .unwrap()
            .raw_os_error(),
        Some(28)
    );
    assert_eq!(spool.metrics.stats().records_durable, 1);
    drop(spool);
    assert_eq!(
        load(&Spool::open(cfg, None, Default::default()).unwrap()).len(),
        1
    );
}

#[test]
fn publication_confirmation_failures_replay_as_pending_or_published() {
    for point in ["publish_confirm_before", "publish_confirm_after"] {
        let dir = tempfile::tempdir().unwrap();
        let cfg = config(dir.path());
        let mut spool =
            Spool::open(cfg.clone(), Some("fixture".into()), Default::default()).unwrap();
        spool.append(&[&datum(1)]).unwrap();
        spool.seal().unwrap();
        let id = spool.segments()[0].id.clone();
        spool.fault = Some((point.into(), false, 5));
        assert!(spool.published(&id).is_err());
        drop(spool);
        let restored = Spool::open(cfg, Some("fixture".into()), Default::default()).unwrap();
        assert_eq!(load(&restored).len(), 1);
        assert_eq!(
            restored.segments()[0].state,
            if point.ends_with("after") {
                Publication::Published
            } else {
                Publication::Pending
            }
        );
    }
}

#[test]
fn all_reference_kinds_block_gc_until_durable_release() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.gc_enabled = true;
    let mut spool = Spool::open(cfg.clone(), Some("fixture".into()), Default::default()).unwrap();
    spool.append(&[&datum(1)]).unwrap();
    spool.seal().unwrap();
    let id = spool.segments()[0].id.clone();
    spool.published(&id).unwrap();
    for kind in ["revisit", "304", "consumer", "outbox", "backup"] {
        spool.reference(&id, "fixture", kind, true).unwrap();
    }
    spool.reference(&id, "archive", "consumer", false).unwrap();
    drop(spool);
    let mut spool = Spool::open(cfg, Some("fixture".into()), Default::default()).unwrap();
    for kind in ["revisit", "304", "consumer", "outbox", "backup"] {
        assert_eq!(spool.gc().unwrap(), 0);
        assert_eq!(load(&spool).len(), 1);
        spool.reference(&id, "fixture", kind, false).unwrap();
    }
    assert_eq!(spool.gc().unwrap(), 1);
    assert!(spool.segments().is_empty());
}

#[test]
fn pending_publication_and_backup_keep_gc_blocked() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.gc_enabled = true;
    let mut spool = Spool::open(cfg, Some("fixture".into()), Default::default()).unwrap();
    spool.append(&[&datum(1)]).unwrap();
    spool.seal().unwrap();
    let id = spool.segments()[0].id.clone();
    spool.reference(&id, "archive", "consumer", false).unwrap();
    assert_eq!(spool.gc().unwrap(), 0);
    spool
        .remember_upload(&id, Some("in-flight".into()))
        .unwrap();
    assert_eq!(spool.gc().unwrap(), 0);
}

#[test]
fn spool_lock_and_target_identity_prevent_concurrent_or_wrong_bucket_reuse() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let mut spool = Spool::open(cfg.clone(), Some("bucket-a".into()), Default::default()).unwrap();
    assert!(Spool::open(cfg.clone(), Some("bucket-a".into()), Default::default()).is_err());
    spool.append(&[&datum(1)]).unwrap();
    spool.seal().unwrap();
    drop(spool);
    assert!(
        Spool::open(cfg, Some("bucket-b".into()), Default::default())
            .err()
            .unwrap()
            .to_string()
            .contains("target changed")
    );
}

#[test]
fn record_count_and_time_rotation_produce_compatible_segments() {
    let dir = tempfile::tempdir().unwrap();
    let mut cfg = config(dir.path());
    cfg.segment_records = 1;
    let mut spool = Spool::open(cfg, None, Default::default()).unwrap();
    spool.append(&[&datum(1), &datum(2)]).unwrap();
    assert_eq!(spool.segments().len(), 2);
    spool.active.as_mut().unwrap().opened = Instant::now() - Duration::from_secs(1000);
    spool.seal_due().unwrap();
    assert_eq!(spool.metrics.stats().segments_open, 0);
    assert_eq!(load(&spool).len(), 2);
}

#[tokio::test]
async fn concurrent_writes_get_local_durable_acks_and_grouped_syncs() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let pipeline = Pipeline::new(Default::default()).unwrap();
    let writer = Arc::new(
        WarcWriter::open(cfg.clone(), None, pipeline.clone(), None)
            .await
            .unwrap(),
    );
    let mut jobs = tokio::task::JoinSet::new();
    for n in 0..4 {
        let writer = writer.clone();
        jobs.spawn(async move { writer.write(datum(n)).await });
    }
    while let Some(result) = jobs.join_next().await {
        result.unwrap().unwrap();
    }
    assert_eq!(writer.metrics.stats().records_durable, 4);
    assert!(writer.metrics.stats().group_commits < 4);
    writer.checkpoint().await.unwrap();
    writer.finish().await.unwrap();
    drop(writer);
    pipeline.shutdown().await;
    let spool = Spool::open(cfg, None, Default::default()).unwrap();
    assert_eq!(load(&spool).len(), 4);
}

#[tokio::test]
async fn business_commit_failure_retains_archive_and_stops_admission() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let pipeline = Pipeline::new(Default::default()).unwrap();
    let writer = WarcWriter::open(
        cfg.clone(),
        None,
        pipeline.clone(),
        Some(Arc::new(|_, _| anyhow::bail!("frontier disk unavailable"))),
    )
    .await
    .unwrap();
    assert!(writer.write(datum(1)).await.is_err());
    assert_eq!(writer.metrics.stats().records_durable, 1);
    assert!(writer.metrics.stats().errors > 0);
    assert!(writer.finish().await.is_err());
    assert!(pipeline.admit(1).await.is_err());
    drop(writer);
    pipeline.shutdown().await;
    let restored = Spool::open(cfg, None, Default::default()).unwrap();
    assert_eq!(load(&restored).len(), 1);
}

#[tokio::test]
async fn accepted_queued_write_survives_waiter_cancellation_and_shutdown_drains() {
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path());
    let pipeline = Pipeline::new(Default::default()).unwrap();
    let writer = Arc::new(
        WarcWriter::open(cfg.clone(), None, pipeline.clone(), None)
            .await
            .unwrap(),
    );
    let (release, blocked) = std::sync::mpsc::channel();
    let blocker = {
        let stage = pipeline.storage.clone();
        tokio::spawn(async move { stage.run_durable(0, move || blocked.recv().unwrap()).await })
    };
    while pipeline.stats().storage.running == 0 {
        tokio::task::yield_now().await;
    }
    let task = {
        let writer = writer.clone();
        tokio::spawn(async move { writer.write(datum(1)).await })
    };
    while pipeline.stats().batch_queue.items == 0 {
        tokio::task::yield_now().await;
    }
    task.abort();
    let _ = task.await;
    release.send(()).unwrap();
    blocker.await.unwrap().unwrap();
    writer.finish().await.unwrap();
    drop(writer);
    pipeline.shutdown().await;
    assert_eq!(pipeline.stats().batch_queue.items, 0);
    let spool = Spool::open(cfg, None, Default::default()).unwrap();
    assert_eq!(load(&spool).len(), 1);
}
