use super::*;

fn observer() -> (tempfile::TempDir, Observer) {
    let directory = tempfile::tempdir().unwrap();
    let now = Utc::now().to_rfc3339();
    let observer = Observer::new(
        directory.path().join("status.json"),
        Snapshot {
            schema_version: 1,
            run_id: "observer-c02-test".into(),
            pid: std::process::id(),
            started_at: now.clone(),
            updated_at: now,
            finished_at: None,
            phase: "running".into(),
            user_agent: "test".into(),
            workers: 1,
            min_crawl_delay_ms: 1,
            pages_per_domain: 5,
            continuous: false,
            frontier: FrontierState::default(),
            counters: Counters::default(),
            sites: Vec::new(),
            recent_events: Vec::new(),
            warc: Warc {
                directory: directory.path().display().to_string(),
                files: 0,
                bytes: 0,
                current_file: None,
            },
            history: Vec::new(),
        },
    )
    .unwrap();
    (directory, observer)
}

fn datum(url: url::Url) -> super::super::CrawlDatum {
    super::super::CrawlDatum {
        url,
        payload_type: crate::warc::PayloadType::Html,
        body: "<html>saved</html>".into(),
        fetch_time_ms: 3,
        date: Utc::now(),
    }
}

#[test]
fn transport_and_explicit_outcomes_remain_separate() {
    let (directory, observer) = observer();
    let url = url::Url::parse("https://example.test/page").unwrap();
    let domain = Domain::from(&url);
    observer.register_site(domain.as_str().into(), 5);
    observer.event(&domain, Some(&url), "fetching", "candidate");
    for _ in 0..3 {
        observer.http_attempt(&domain, &url);
    }
    observer.redirect(&domain, &url);
    observer.saved(&domain, &datum(url.clone()), "saved.warc.gz".into(), 12);
    for kind in [
        "failed",
        "skipped",
        "deferred",
        "cancelled",
        "storage_failed",
    ] {
        observer.event(&domain, Some(&url), kind, kind);
    }
    observer.flush().unwrap();
    observer.check().unwrap();
    let snapshot: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.path().join("status.json")).unwrap()).unwrap();
    let counters = &snapshot["counters"];
    assert_eq!(counters["attempted"], 1);
    assert_eq!(counters["page_http_attempts"], 3);
    assert_eq!(counters["redirect_hops"], 1);
    assert_eq!(counters["downloaded"], 1);
    assert_eq!(counters["useful_pages"], 1);
    for kind in [
        "failed",
        "skipped",
        "deferred",
        "cancelled",
        "storage_failed",
    ] {
        assert_eq!(counters[kind], 1);
        assert_eq!(snapshot["sites"][0][kind], 1);
    }
}

#[test]
fn finish_preserves_incomplete_outcomes_and_unknown_saved_domains_are_safe() {
    let (_directory, observer) = observer();
    for status in ["deferred", "cancelled", "storage_failed"] {
        let url = url::Url::parse(&format!("https://{status}.test/page")).unwrap();
        let domain = Domain::from(&url);
        observer.register_site(domain.as_str().into(), 5);
        observer.event(&domain, Some(&url), &format!("site_{status}"), status);
    }
    let url = url::Url::parse("https://unregistered.test/page").unwrap();
    observer.saved(
        &Domain::from(&url),
        &datum(url),
        "redirect.warc.gz".into(),
        12,
    );
    observer.finish("stopped");
    observer.flush().unwrap();
    observer.check().unwrap();
    let state = observer.shared.state.lock().unwrap();
    let snapshot = &state.snapshot;
    assert_eq!(snapshot.counters.downloaded, 1);
    assert_eq!(snapshot.counters.useful_pages, 1);
    assert_eq!(snapshot.counters.completed_sites, 0);
    for (site, expected) in snapshot
        .sites
        .iter()
        .zip(["deferred", "cancelled", "storage_failed"])
    {
        assert_eq!(site.status, expected);
        assert!(site.current_url.is_none());
    }
}

fn snapshot(directory: &Path) -> Snapshot {
    let now = Utc::now().to_rfc3339();
    Snapshot {
        schema_version: 1,
        run_id: "observer-c03-test".into(),
        pid: std::process::id(),
        started_at: now.clone(),
        updated_at: now,
        finished_at: None,
        phase: "running".into(),
        user_agent: "test".into(),
        workers: 1,
        min_crawl_delay_ms: 1,
        pages_per_domain: 5,
        continuous: false,
        frontier: FrontierState::default(),
        counters: Counters::default(),
        sites: Vec::new(),
        recent_events: Vec::new(),
        warc: Warc {
            directory: directory.display().to_string(),
            files: 0,
            bytes: 0,
            current_file: None,
        },
        history: Vec::new(),
    }
}

#[test]
fn slow_projection_does_not_block_producers_or_drop_totals() {
    use std::sync::{mpsc, Barrier};
    let directory = tempfile::tempdir().unwrap();
    let blocked = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let first = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let publisher: Arc<Publisher> = {
        let blocked = blocked.clone();
        let release = release.clone();
        Arc::new(move |path, snapshot, counters, legacy| {
            if first.swap(false, Ordering::SeqCst) {
                blocked.wait();
                release.wait();
            }
            persist(path, snapshot, counters, legacy)
        })
    };
    let observer = Arc::new(
        Observer::create(
            directory.path().join("status.json"),
            snapshot(directory.path()),
            Duration::from_millis(20),
            8,
            false,
            publisher,
        )
        .unwrap(),
    );
    blocked.wait();
    let (done_tx, done_rx) = mpsc::channel();
    let producer = observer.clone();
    let handle = thread::spawn(move || {
        let url = url::Url::parse("https://example.test/page").unwrap();
        let domain = Domain::from(&url);
        producer.register_site(domain.as_str().into(), 1000);
        for _ in 0..1000 {
            producer.http_attempt(&domain, &url);
            producer.saved(&domain, &datum(url.clone()), "saved.warc.gz".into(), 12);
        }
        done_tx.send(()).unwrap();
    });
    let completed_while_blocked = done_rx.recv_timeout(Duration::from_secs(2)).is_ok();
    let while_blocked = observer.statistics();
    let bounded_flush = observer.flush_timeout(Duration::from_millis(20));
    release.wait();
    handle.join().unwrap();
    assert!(
        completed_while_blocked,
        "producer waited for the blocked writer"
    );
    assert!(bounded_flush.is_err());
    assert_eq!(while_blocked.events_total, 2000);
    assert_eq!(while_blocked.events_dropped, 1992);
    assert_eq!(while_blocked.snapshot_writes, 0);
    observer.finish("completed");
    let stats = observer.statistics();
    assert_eq!(stats.pending_updates, 0);
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(directory.path().join("status.json")).unwrap()).unwrap();
    assert_eq!(saved["counters"]["downloaded"], 1000);
    assert_eq!(saved["counters"]["page_http_attempts"], 1000);
    assert_eq!(saved["recent_events"].as_array().unwrap().len(), 8);
    assert_eq!(saved["phase"], "completed");
}

#[test]
fn unwritable_metrics_are_reconstructible_and_recover_without_losing_totals() {
    let directory = tempfile::tempdir().unwrap();
    let blocked_directory = directory.path().join("blocked");
    fs::write(&blocked_directory, b"not a directory").unwrap();
    let path = blocked_directory.join("status.json");
    let observer = Observer::new(path.clone(), snapshot(directory.path())).unwrap();
    let url = url::Url::parse("https://example.test/page").unwrap();
    let domain = Domain::from(&url);
    observer.saved(&domain, &datum(url), "safe.warc.gz".into(), 12);
    assert!(observer.flush().is_err());
    let failed = observer.statistics();
    assert_eq!(failed.snapshot_writes, 0);
    assert!(failed.errors >= 1);
    assert!(observer.check().is_err());
    assert_eq!(
        observer
            .shared
            .state
            .lock()
            .unwrap()
            .snapshot
            .counters
            .downloaded,
        1
    );
    fs::remove_file(&blocked_directory).unwrap();
    observer.flush().unwrap();
    observer.check().unwrap();
    let recovered = observer.statistics();
    assert_eq!(recovered.pending_updates, 0);
    assert!(recovered.last_error.is_none());
    assert!(recovered.last_failure_at.is_some());
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(saved["counters"]["downloaded"], 1);
    assert!(saved["telemetry"]["last_error"].is_null());
}

#[test]
fn failed_publication_keeps_previous_snapshot_and_reports_actual_writes() {
    let directory = tempfile::tempdir().unwrap();
    let (directory, observer) = {
        let observer = Observer::new(
            directory.path().join("status.json"),
            snapshot(directory.path()),
        )
        .unwrap();
        (directory, observer)
    };
    observer.flush().unwrap();
    let path = directory.path().join("status.json");
    let initial = fs::read(&path).unwrap();
    let writes = observer.statistics().snapshot_writes;
    fs::create_dir(path.with_extension("json.tmp")).unwrap();
    observer.phase("changed");
    assert!(observer.flush().is_err());
    assert_eq!(fs::read(&path).unwrap(), initial);
    assert_eq!(observer.statistics().snapshot_writes, writes);
    fs::remove_dir(path.with_extension("json.tmp")).unwrap();
    observer.flush().unwrap();
    assert_eq!(observer.statistics().snapshot_writes, writes + 1);
    let saved: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(saved["phase"], "changed");
    assert_eq!(
        observer.statistics().write_calls,
        observer.statistics().snapshot_writes
    );
}

#[test]
fn changed_counters_become_visible_without_explicit_flush_within_five_seconds() {
    let (directory, observer) = observer();
    observer.flush().unwrap();
    let url = url::Url::parse("https://example.test/page").unwrap();
    observer.saved(&Domain::from(&url), &datum(url), "safe.warc.gz".into(), 12);
    let started = Instant::now();
    loop {
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("status.json")).unwrap())
                .unwrap();
        if saved["counters"]["downloaded"] == 1 {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(10));
    }
    assert!(observer.statistics().max_publish_latency_ms <= 5000);
}

#[test]
fn site_ring_is_bounded_and_evicted_sites_still_contribute_global_totals() {
    let (_directory, observer) = observer();
    for i in 0..600 {
        observer.register_site(format!("site{i}.test"), 5);
    }
    let url = url::Url::parse("https://site0.test/page").unwrap();
    let domain = Domain::from(&url);
    observer.http_attempt(&domain, &url);
    observer.response(&domain, 200);
    observer.waiting(&domain, Duration::MAX);
    observer.saved(&domain, &datum(url), "safe.warc.gz".into(), 12);
    observer.finish("stopped");
    let state = observer.shared.state.lock().unwrap();
    assert_eq!(state.snapshot.sites.len(), 500);
    assert_eq!(state.snapshot.counters.sites, 600);
    assert_eq!(state.snapshot.counters.page_http_attempts, 1);
    assert_eq!(state.snapshot.counters.downloaded, 1);
}

#[test]
fn batched_io_removes_at_least_ninety_percent_of_real_legacy_writes() {
    let directory = tempfile::tempdir().unwrap();
    let legacy = Observer::new_legacy(
        directory.path().join("legacy.json"),
        snapshot(directory.path()),
    )
    .unwrap();
    let batched = Observer::new(
        directory.path().join("batched.json"),
        snapshot(directory.path()),
    )
    .unwrap();
    let url = url::Url::parse("https://example.test/page").unwrap();
    let domain = Domain::from(&url);
    for observer in [&legacy, &batched] {
        observer.register_site(domain.as_str().into(), 50);
        for _ in 0..50 {
            observer.http_attempt(&domain, &url);
            observer.saved(&domain, &datum(url.clone()), "safe.warc.gz".into(), 12);
        }
        observer.finish("completed");
    }
    let before = legacy.statistics();
    let after = batched.statistics();
    assert_eq!(before.updates, after.updates);
    assert_eq!(before.events_total, after.events_total);
    assert_eq!(before.events_dropped, after.events_dropped);
    assert!(after.write_calls * 10 <= before.write_calls);
    assert!(after.snapshot_writes * 10 <= before.snapshot_writes);
    for filename in ["legacy.json", "batched.json"] {
        let saved: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join(filename)).unwrap()).unwrap();
        assert_eq!(saved["counters"]["page_http_attempts"], 50);
        assert_eq!(saved["counters"]["downloaded"], 50);
    }
}

#[test]
fn rapid_heartbeats_keep_history_window_and_drop_never_waits_for_blocked_output() {
    let (_directory, observer) = observer();
    for _ in 0..1000 {
        observer.heartbeat();
    }
    assert_eq!(observer.shared.state.lock().unwrap().rings.history.len(), 1);
    assert_eq!(observer.statistics().updates, 1000);
    // The blocking-writer test above exercises bounded flush; Drop contains no join wait.
}

#[test]
fn drop_is_bounded_even_when_projection_cannot_make_progress() {
    use std::sync::{mpsc, Barrier};
    let directory = tempfile::tempdir().unwrap();
    let blocked = Arc::new(Barrier::new(2));
    let release = Arc::new(Barrier::new(2));
    let publisher: Arc<Publisher> = {
        let blocked = blocked.clone();
        let release = release.clone();
        Arc::new(move |path, snapshot, counters, legacy| {
            blocked.wait();
            release.wait();
            persist(path, snapshot, counters, legacy)
        })
    };
    let observer = Observer::create(
        directory.path().join("status.json"),
        snapshot(directory.path()),
        Duration::from_secs(1),
        8,
        false,
        publisher,
    )
    .unwrap();
    blocked.wait();
    let (tx, rx) = mpsc::channel();
    let handle = thread::spawn(move || {
        drop(observer);
        tx.send(()).unwrap();
    });
    let dropped = rx.recv_timeout(Duration::from_secs(1)).is_ok();
    release.wait();
    handle.join().unwrap();
    assert!(
        dropped,
        "dropping telemetry must not wait for a blocked filesystem"
    );
}

#[test]
fn completion_totals_remain_exact_when_old_sites_are_evicted_and_registered_again() {
    let (_directory, observer) = observer();
    let url = url::Url::parse("https://old.test/page").unwrap();
    let domain = Domain::from(&url);
    observer.register_site(domain.as_str().into(), 5);
    observer.event(&domain, None, "site_completed", "done");
    for i in 0..500 {
        observer.register_site(format!("site{i}.test"), 5);
    }
    {
        let state = observer.shared.state.lock().unwrap();
        assert!(!state
            .snapshot
            .sites
            .iter()
            .any(|site| site.host == domain.as_str()));
        assert_eq!(state.snapshot.counters.completed_sites, 1);
    }
    observer.register_site(domain.as_str().into(), 5);
    assert_eq!(
        observer
            .shared
            .state
            .lock()
            .unwrap()
            .snapshot
            .counters
            .completed_sites,
        0
    );
    observer.event(&domain, None, "site_deferred", "retry later");
    assert_eq!(
        observer
            .shared
            .state
            .lock()
            .unwrap()
            .snapshot
            .counters
            .completed_sites,
        0
    );
    observer.event(&domain, None, "site_completed", "done");
    observer.event(&domain, None, "site_completed", "repeated notification");
    observer.finish("stopped");
    let state = observer.shared.state.lock().unwrap();
    assert_eq!(state.snapshot.counters.completed_sites, 1);
    assert_eq!(state.snapshot.counters.sites, 501);
}
