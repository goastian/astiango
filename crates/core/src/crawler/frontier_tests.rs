use super::*;

fn open(path: &Path) -> Frontier {
    Frontier::open_database(path.join("frontier.json"), 256, 10).unwrap()
}

fn receipt(capture: &crate::capture::Capture) -> Receipt {
    Receipt {
        capture_id: capture.metadata.fetch_id.clone(),
        capture_version: Some(1),
        record_offset: Some(100),
        segment: "test.warc.gz".into(),
        start: 0,
        end: 500,
        sha256: "checksum".into(),
    }
}

#[test]
fn committed_urls_and_discoveries_survive_without_replaying_success() {
    let directory = tempfile::tempdir().unwrap();
    let frontier = open(directory.path());
    let url = Url::parse("https://example.org/a?utm_source=keep").unwrap();
    frontier.discover([url.clone()]).unwrap();
    let job = frontier.claim().unwrap().unwrap();
    let id = frontier.begin_url(&url).unwrap().unwrap();
    let mut capture = crate::capture::fixture(&url, id, b"body".to_vec(), Vec::new());
    let internal = Url::parse("https://example.org/b").unwrap();
    let external = Url::parse("https://external.org/c").unwrap();
    capture.metadata.internal_urls.push(internal.clone());
    capture.metadata.discovery_urls.push(external.clone());
    let receipt = receipt(&capture);
    assert!(frontier
        .commit_capture(&capture.metadata, &receipt, false)
        .unwrap());
    assert!(!frontier
        .commit_capture(&capture.metadata, &receipt, false)
        .unwrap());
    assert_eq!(frontier.metrics.snapshot().outbox_entries, 1);
    drop(frontier);
    let restored = open(directory.path());
    assert_eq!(restored.metrics.snapshot().outbox_entries, 0);
    assert!(restored.begin_url(&url).unwrap().is_none());
    let replay = restored.claim().unwrap().unwrap();
    assert_eq!(replay.domain, job.domain);
    assert_eq!(replay.urls[0].url, internal);
    assert_eq!(restored.claim().unwrap().unwrap().urls[0].url, external);
    assert_eq!(restored.metrics.snapshot().commits, 1);
}

#[test]
fn unfinished_url_lease_keeps_fetch_identity_across_restart() {
    let directory = tempfile::tempdir().unwrap();
    let frontier = open(directory.path());
    let url = Url::parse("https://example.org/a").unwrap();
    frontier.discover([url.clone()]).unwrap();
    frontier.claim().unwrap();
    let id = frontier.begin_url(&url).unwrap().unwrap();
    assert!(frontier.begin_url(&url).unwrap().is_none());
    drop(frontier);
    let restored = open(directory.path());
    assert_eq!(restored.begin_url(&url).unwrap().unwrap(), id);
    assert_eq!(restored.metrics.snapshot().recovered_leases, 1);
}

#[test]
fn streaming_migration_preserves_original_and_delayed_jobs() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("frontier.json");
    let url = Url::parse("https://example.org/a").unwrap();
    let job = Job {
        domain: Domain::from(&url),
        urls: VecDeque::from([WeightedUrl {
            url: url.clone(),
            weight: 1.0,
        }]),
        wandering_urls: 9,
    };
    let due = chrono::Utc::now() + chrono::Duration::hours(1);
    let original = serde_json::to_vec(&serde_json::json!({"seen":["example.org"], "pending":[], "active":{}, "delayed":[{"job":job,"not_before":due}]})).unwrap();
    fs::write(&path, &original).unwrap();
    let frontier = open(directory.path());
    assert_eq!(fs::read(&path).unwrap(), original);
    assert_eq!(
        fs::read(path.with_extension("json.pre-c10")).unwrap(),
        original
    );
    assert!(frontier
        .claim_at(due - chrono::Duration::seconds(1))
        .unwrap()
        .is_none());
    assert_eq!(frontier.claim_at(due).unwrap().unwrap().urls[0].url, url);
    assert!(frontier.metrics.snapshot().migration_complete);
}

#[test]
fn durable_archive_reconciles_fetch_id_and_outbox_after_lost_commit() {
    let directory = tempfile::tempdir().unwrap();
    let frontier = open(directory.path());
    let url = Url::parse("https://example.org/a").unwrap();
    frontier.discover([url.clone()]).unwrap();
    frontier.claim().unwrap();
    let mut capture = crate::capture::fixture(
        &url,
        frontier.begin_url(&url).unwrap().unwrap(),
        b"body".to_vec(),
        Vec::new(),
    );
    capture
        .metadata
        .discovery_urls
        .push(Url::parse("https://external.org/").unwrap());
    let datum = super::super::CrawlDatum {
        url: url.clone(),
        body: "body".into(),
        payload_type: crate::warc::PayloadType::Html,
        fetch_time_ms: 1,
        date: capture.metadata.response.received_at,
        capture: Some(capture),
    };
    let settings = super::super::spool::SpoolConfig {
        directory: directory.path().join("spool"),
        ..Default::default()
    };
    let mut spool =
        super::super::spool::Spool::open(settings.clone(), None, Default::default()).unwrap();
    spool.append(&[&datum]).unwrap();
    drop(spool);
    drop(frontier);
    let spool =
        super::super::spool::Spool::open(settings.clone(), None, Default::default()).unwrap();
    let restored = open(directory.path());
    restored.reconcile_spool(&settings.directory).unwrap();
    assert!(restored.begin_url(&url).unwrap().is_none());
    assert_eq!(restored.metrics.snapshot().commits, 1);
    assert_eq!(restored.metrics.snapshot().reconciled_captures, 1);
    assert_eq!(restored.metrics.snapshot().outbox_entries, 0);
    assert_eq!(restored.metrics.snapshot().known_sites, 2);
    restored.reconcile_spool(&settings.directory).unwrap();
    assert_eq!(restored.metrics.snapshot().commits, 1);
    drop(spool);
}

#[test]
fn terminal_and_deferred_urls_are_persisted_separately() {
    let directory = tempfile::tempdir().unwrap();
    let frontier = open(directory.path());
    let a = Url::parse("https://example.org/a").unwrap();
    let b = Url::parse("https://example.org/b").unwrap();
    frontier.admit_urls([a.clone(), b.clone()]).unwrap();
    let job = frontier.claim().unwrap().unwrap();
    frontier.begin_url(&a).unwrap();
    frontier.begin_url(&b).unwrap();
    let mut outcome = JobOutcome::default();
    outcome.record(
        a,
        FetchOutcome::PermanentFailure {
            reason: "404".into(),
        },
    );
    outcome.record(
        b.clone(),
        FetchOutcome::Deferred {
            reason: "429".into(),
            retry_after_ms: Some(60_000),
        },
    );
    let now = chrono::Utc::now();
    frontier
        .resolve(&job.domain, &outcome, 10_000, now)
        .unwrap();
    assert_eq!(frontier.metrics.snapshot().terminal_urls, 1);
    assert_eq!(frontier.metrics.snapshot().leased_urls, 0);
    drop(frontier);
    let restored = open(directory.path());
    assert!(restored
        .claim_at(now + chrono::Duration::seconds(59))
        .unwrap()
        .is_none());
    assert_eq!(
        restored
            .claim_at(now + chrono::Duration::seconds(60))
            .unwrap()
            .unwrap()
            .urls[0]
            .url,
        b
    );
}
