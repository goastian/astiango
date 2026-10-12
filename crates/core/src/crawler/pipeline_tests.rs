use super::*;
use std::sync::{atomic::AtomicUsize, mpsc as sync_mpsc};

fn pipeline() -> Pipeline {
    Pipeline::new(PipelineConfig {
        max_inflight_documents: 2,
        max_inflight_bytes: 16,
        parse: StageConfig {
            workers: 1,
            max_items: 2,
            max_bytes: 8,
        },
        storage: StageConfig {
            workers: 1,
            max_items: 2,
            max_bytes: 8,
        },
        queue_wait_timeout_ms: 500,
        ..PipelineConfig::default()
    })
    .unwrap()
}

#[tokio::test]
async fn capture_retains_robots_truncation_and_total_rules_across_the_parse_stage() {
    let pipeline = Pipeline::new(PipelineConfig::default()).unwrap();
    let reader = super::super::body::BodyReader::new(Default::default()).unwrap();
    let url = url::Url::parse("https://example.org/").unwrap();
    let mut body = "<html><head>".to_owned();
    for _ in 0..1000 {
        body.push_str("<meta name=AstianGOBot content=index>");
    }
    body.push_str("<meta name=AstianGOBot content=none></head><body>Original</body></html>");
    let response = http_02::Response::builder()
        .header("content-type", "text/html; charset=UTF-8")
        .body(reqwest::Body::from(body.clone()))
        .unwrap()
        .into();
    let (_, body_lease) = reader
        .read(response, super::super::body::BodyKind::Html)
        .await
        .unwrap()
        .into_parts();
    let capture = crate::capture::fixture(
        &url,
        uuid::Uuid::new_v4().to_string(),
        body.as_bytes().to_vec(),
        vec![
            crate::capture::Header {
                name: "content-type".into(),
                value_hex: crate::capture::hex(b"text/html; charset=UTF-8"),
            },
            crate::capture::Header {
                name: "x-robots-tag".into(),
                value_hex: crate::capture::hex(b"index"),
            },
        ],
    );
    let datum = super::super::CrawlDatum {
        url: url.clone(),
        body,
        payload_type: crate::warc::PayloadType::Html,
        fetch_time_ms: 1,
        date: chrono::Utc::now(),
        capture: Some(capture),
    };
    let admission = pipeline.admit(datum.memory_bytes()).await.unwrap();
    let prepared = pipeline
        .prepare_page(
            datum,
            body_lease,
            admission,
            super::super::extraction::Extractor::default(),
            super::super::Domain::from(&url),
            Arc::new(Default::default()),
            url,
        )
        .await
        .unwrap();
    let robots = &prepared.datum.capture.as_ref().unwrap().metadata.robots;
    assert_eq!(robots.directives_seen, 1002);
    assert_eq!(robots.directives.len(), 64);
    assert!(robots.directives_truncated && robots.no_index && robots.no_follow);
    drop(prepared);
    pipeline.shutdown().await;
}

#[tokio::test]
async fn a_stage_reuses_its_fixed_workers_instead_of_spreading_allocations_over_new_threads() {
    let pipeline = pipeline();
    let threads = Arc::new(Mutex::new(std::collections::HashSet::new()));
    for _ in 0..100 {
        let ids = threads.clone();
        pipeline
            .parse
            .run(1, move || {
                ids.lock().unwrap().insert(std::thread::current().id());
            })
            .await
            .unwrap();
    }
    pipeline.shutdown().await;
    assert_eq!(threads.lock().unwrap().len(), 1);
    assert_eq!(pipeline.stats().parse.completed, 100);
}
async fn wait_until(mut predicate: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while !predicate() {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "current_thread")]
async fn saturated_storage_stops_at_count_and_bytes_without_blocking_control() {
    let pipeline = pipeline();
    let (release, blocked) = sync_mpsc::channel();
    let first = {
        let stage = pipeline.storage.clone();
        tokio::spawn(async move { stage.run_durable(4, move || blocked.recv().unwrap()).await })
    };
    wait_until(|| pipeline.storage.stats().running == 1).await;
    let second = {
        let stage = pipeline.storage.clone();
        tokio::spawn(async move { stage.run_durable(4, || 2).await })
    };
    wait_until(|| pipeline.storage.stats().items == 2).await;
    let third = {
        let stage = pipeline.storage.clone();
        tokio::spawn(async move { stage.run_durable(1, || 3).await })
    };
    wait_until(|| pipeline.storage.stats().admission_waits > 0).await;
    assert_eq!(pipeline.storage.stats().submitted, 2);
    assert_eq!(pipeline.storage.stats().bytes, 8);
    let control = tokio::time::timeout(
        Duration::from_millis(100),
        pipeline.control.run_durable(1, || 7),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(control, 7);
    assert!(pipeline.storage.stats().peak_running <= 1);
    release.send(()).unwrap();
    first.await.unwrap().unwrap();
    second.await.unwrap().unwrap();
    third.await.unwrap().unwrap();
    pipeline.shutdown().await;
    assert_eq!(pipeline.storage.stats().items, 0);
    assert_eq!(pipeline.storage.stats().bytes, 0);
}

#[tokio::test]
async fn byte_budget_limits_queue_independently_of_message_capacity() {
    let pipeline = pipeline();
    let (release, blocked) = sync_mpsc::channel();
    let first = {
        let stage = pipeline.parse.clone();
        tokio::spawn(async move { stage.run(8, move || blocked.recv().unwrap()).await })
    };
    wait_until(|| pipeline.parse.stats().running == 1).await;
    let second = {
        let stage = pipeline.parse.clone();
        tokio::spawn(async move { stage.run(1, || 1).await })
    };
    wait_until(|| pipeline.parse.stats().admission_waits == 1).await;
    assert_eq!(pipeline.parse.stats().items, 1);
    assert_eq!(pipeline.parse.stats().submitted, 1);
    second.abort();
    let _ = second.await;
    release.send(()).unwrap();
    first.await.unwrap().unwrap();
    pipeline.shutdown().await;
    assert_eq!(pipeline.parse.stats().bytes, 0);
}

#[tokio::test]
async fn cancelling_running_work_keeps_resources_until_completion() {
    let pipeline = pipeline();
    let admission = pipeline.admit(8).await.unwrap();
    let (release, blocked) = sync_mpsc::channel();
    let running = {
        let stage = pipeline.parse.clone();
        tokio::spawn(async move {
            stage
                .run(4, move || {
                    let _admission = admission;
                    blocked.recv().unwrap()
                })
                .await
        })
    };
    wait_until(|| pipeline.parse.stats().running == 1).await;
    running.abort();
    let _ = running.await;
    assert_eq!(pipeline.stats().admission.items, 1);
    assert_eq!(pipeline.parse.stats().running, 1);
    release.send(()).unwrap();
    pipeline.shutdown().await;
    assert_eq!(pipeline.stats().admission.items, 0);
}

#[tokio::test]
async fn queued_cancelled_parse_is_skipped_but_accepted_storage_is_drained() {
    for durable in [false, true] {
        let pipeline = pipeline();
        let (release, blocked) = sync_mpsc::channel();
        let stage = if durable {
            pipeline.storage.clone()
        } else {
            pipeline.parse.clone()
        };
        let first = {
            let stage = stage.clone();
            tokio::spawn(async move { stage.run_durable(4, move || blocked.recv().unwrap()).await })
        };
        wait_until(|| stage.stats().running == 1).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let second = {
            let stage = stage.clone();
            let calls = calls.clone();
            tokio::spawn(async move {
                let work = move || {
                    calls.fetch_add(1, Ordering::Relaxed);
                };
                if durable {
                    stage.run_durable(4, work).await
                } else {
                    stage.run(4, work).await
                }
            })
        };
        wait_until(|| stage.stats().items == 2).await;
        second.abort();
        let _ = second.await;
        release.send(()).unwrap();
        first.await.unwrap().unwrap();
        pipeline.shutdown().await;
        assert_eq!(calls.load(Ordering::Relaxed), usize::from(durable));
        assert_eq!(stage.stats().cancelled, u64::from(!durable));
    }
}

#[tokio::test]
async fn shutdown_wakes_admission_waiters_and_rejects_new_stage_work() {
    let pipeline = pipeline();
    let first = pipeline.admit(16).await.unwrap();
    let waiting = {
        let pipeline = pipeline.clone();
        tokio::spawn(async move { pipeline.admit(1).await })
    };
    wait_until(|| pipeline.stats().admission.admission_waits == 1).await;
    pipeline.stop_admission();
    assert!(matches!(waiting.await.unwrap(), Err(PipelineError::Closed)));
    drop(first);
    pipeline.shutdown().await;
    assert!(matches!(
        pipeline.parse.run(1, || 1).await,
        Err(PipelineError::Closed)
    ));
}

#[tokio::test]
async fn panic_and_oversize_release_budget_and_do_not_kill_the_pool() {
    let pipeline = pipeline();
    assert!(matches!(
        pipeline.parse.run(9, || 1).await,
        Err(PipelineError::TooLarge { .. })
    ));
    assert!(matches!(
        pipeline
            .parse
            .run(4, || panic!("controlled parser panic"))
            .await,
        Err(PipelineError::WorkerFailed)
    ));
    assert_eq!(pipeline.parse.run(4, || 9).await.unwrap(), 9);
    assert_eq!(pipeline.parse.stats().failed, 1);
    pipeline.shutdown().await;
    assert_eq!(pipeline.parse.stats().bytes, 0);
}

#[test]
fn configuration_defaults_and_invalid_limits_are_explicit() {
    let config: PipelineConfig = serde_json::from_str("{}").unwrap();
    config
        .validate_body(&super::super::body::BodyConfig::default())
        .unwrap();
    let mut bad = config.clone();
    bad.parse.max_items = 0;
    assert!(bad.validate().is_err());
    let mut bad = config.clone();
    bad.storage.workers = 0;
    assert!(bad.validate().is_err());
    let mut bad = config;
    bad.max_inflight_bytes = 1;
    assert!(bad
        .validate_body(&super::super::body::BodyConfig::default())
        .is_err());
    assert!(serde_json::from_str::<PipelineConfig>("{\"inline\":true}").is_err());
}

#[tokio::test]
async fn admission_timeout_releases_no_unowned_capacity() {
    let mut options = PipelineConfig::default();
    options.queue_wait_timeout_ms = 20;
    options.max_inflight_documents = 1;
    options.max_inflight_bytes = 8;
    let pipeline = Pipeline::new(options).unwrap();
    let guard = pipeline.admit(8).await.unwrap();
    assert!(matches!(
        pipeline.admit(1).await,
        Err(PipelineError::QueueTimeout)
    ));
    assert_eq!(pipeline.stats().admission.queue_timeouts, 1);
    assert_eq!(pipeline.stats().admission.items, 1);
    drop(guard);
    pipeline.shutdown().await;
}
