use super::*;

fn open(path: &Path) -> Frontier {
    Frontier::open_database(path.join("frontier.json"), 256, 10).unwrap()
}

#[test]
fn site_identity_migration_splits_tenants_and_preserves_leases_and_commits() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("frontier.redb");
    let leased = "https://alice.github.io/a?sig=Keep%2B&x=1&x=2";
    let committed = "https://alice.github.io/archived";
    let pending = "https://bob.github.io/pending/";
    let db = Database::create(&path).unwrap();
    let tx = db.begin_write().unwrap();
    for table in [
        URLS,
        COMMITS,
        OUTBOX,
        META,
        TableDefinition::new("origins.v1"),
    ] {
        tx.open_table(table).unwrap();
    }
    tx.open_table(LEASES)
        .unwrap()
        .insert("preserved-lease", leased)
        .unwrap();
    tx.open_table(TableDefinition::<(&str, &str), u64>::new(
        "url_eligibility.v1",
    ))
    .unwrap()
    .insert(("github.io", pending), 0)
    .unwrap();
    tx.open_table(TableDefinition::<(u64, u64), &str>::new(
        "origin_eligibility.v1",
    ))
    .unwrap();
    tx.open_table(TableDefinition::<&str, ()>::new("active_origins.v1"))
        .unwrap()
        .insert("github.io", ())
        .unwrap();
    let stats = Stats {
        schema_version: 1,
        identity_version: 1,
        site_identity_version: 1,
        round: 1,
        urls: 3,
        pending_urls: 1,
        leased_urls: 1,
        committed_urls: 1,
        known_sites: 1,
        active_sites: 1,
        commits: 1,
        migration_complete: true,
        ..Default::default()
    };
    put(&tx, META, "stats", &stats).unwrap();
    let due = (chrono::Utc::now() + chrono::Duration::hours(1)).timestamp_millis() as u64;
    put(
        &tx,
        LEGACY_ORIGINS,
        "github.io",
        &OriginRow {
            sequence: 1,
            round: 1,
            state: "pending".into(),
            seed: pending.into(),
            admitted: 3,
            done: 1,
            not_before: due,
        },
    )
    .unwrap();
    for (url, state, id) in [
        (leased, State::Leased, Some("preserved-lease")),
        (committed, State::Committed, Some("preserved-commit")),
        (pending, State::Pending, None),
    ] {
        put(
            &tx,
            URLS,
            url,
            &UrlRow {
                url: url.into(),
                origin: Url::parse(url).unwrap().origin().ascii_serialization(),
                site: "github.io".into(),
                round: 1,
                state,
                fetch_id: id.map(str::to_owned),
                reason: None,
                not_before: 0,
                sitemap: false,
                continuation: None,
            },
        )
        .unwrap();
    }
    put(&tx, COMMITS, "preserved-commit", &serde_json::json!({"url": committed, "receipt": {"capture_id":"preserved-commit", "segment":"existing.warc.gz", "start":0,"end":500,"sha256":"preserved"}})).unwrap();
    tx.commit().unwrap();
    drop(db);
    let original = fs::read(&path).unwrap();
    let frontier = open(directory.path());
    let stats = frontier.metrics.snapshot();
    assert_eq!(
        (
            stats.urls,
            stats.pending_urls,
            stats.committed_urls,
            stats.commits
        ),
        (3, 2, 1, 1)
    );
    assert_eq!(
        (
            stats.known_sites,
            stats.pending_sites,
            stats.identity_mapped_urls
        ),
        (2, 2, 3)
    );
    assert_eq!(stats.site_identity_version, 2);
    assert!(frontier
        .claim_turn_at(chrono::Utc::now())
        .unwrap()
        .is_none());
    assert_eq!(frontier.next_eligible_at().unwrap(), Some(due));
    assert_eq!(
        fs::read(path.with_extension("identity-v1.redb")).unwrap(),
        original
    );
    assert_eq!(
        frontier
            .begin_url(&Url::parse(leased).unwrap())
            .unwrap()
            .as_deref(),
        Some("preserved-lease")
    );
    assert!(frontier.has_commit("preserved-commit").unwrap());
    let tx = frontier.db.begin_read().unwrap();
    let map = tx.open_table(IDENTITY_MAP).unwrap();
    let correspondence: serde_json::Value =
        serde_json::from_slice(map.get(leased).unwrap().unwrap().value()).unwrap();
    assert_eq!(correspondence["old_site"], "github.io");
    assert_eq!(correspondence["new_site"], "alice.github.io");
    assert_eq!(correspondence["fetch_id"], "preserved-lease");
}

#[test]
fn redirect_and_sitemap_continuations_survive_restart_without_changing_fetch_id() {
    let directory = tempfile::tempdir().unwrap();
    let frontier = open(directory.path());
    let url = Url::parse("https://alice.github.io/redirect?sig=A%2B").unwrap();
    let sitemap = Url::parse("https://alice.github.io/sitemap.xml").unwrap();
    frontier.admit_urls([url.clone()]).unwrap();
    frontier.admit_sitemaps(vec![sitemap.clone()]).unwrap();
    let id = frontier.begin_url(&url).unwrap().unwrap();
    let continuation = super::super::RedirectContinuation {
        url: url.join("final/").unwrap(),
        redirects: vec![
            crate::capture::fixture(&url, id.clone(), Vec::new(), Vec::new())
                .metadata
                .response,
        ],
    };
    frontier
        .save_redirect_continuation(&url, continuation.clone())
        .unwrap();
    drop(frontier);
    let restored = open(directory.path());
    assert_eq!(restored.begin_url(&url).unwrap().unwrap(), id);
    assert_eq!(
        restored
            .redirect_continuation(&url)
            .unwrap()
            .unwrap()
            .redirects,
        continuation.redirects
    );
    assert!(restored.is_sitemap(&sitemap).unwrap());
    assert_eq!(restored.metrics.snapshot().urls, 2);
}

#[test]
fn fair_turns_limit_large_sites_and_preserve_signed_urls_and_canonical_alternatives() {
    let directory = tempfile::tempdir().unwrap();
    let frontier =
        Frontier::open_database(directory.path().join("frontier.json"), 256, 10000).unwrap();
    let a = Url::parse("https://alice.github.io/a?sig=A%2B&x=1&x=2").unwrap();
    let b = Url::parse("https://bob.github.io/b/").unwrap();
    frontier.admit_urls([a.clone(), b.clone()]).unwrap();
    frontier
        .admit_urls((0..1000).map(|i| Url::parse(&format!("https://alice.github.io/{i}")).unwrap()))
        .unwrap();
    let job = frontier.claim_turn_at(chrono::Utc::now()).unwrap().unwrap();
    assert_eq!(job.urls.len(), 1);
    assert_eq!(job.wandering_urls, 0);
    let outcome = JobOutcome::default();
    frontier
        .resolve_turn(&job.domain, &outcome, 1000, chrono::Utc::now())
        .unwrap();
    let other = frontier.claim_turn_at(chrono::Utc::now()).unwrap().unwrap();
    assert_ne!(other.domain, job.domain);
    assert_eq!(other.urls[0].url, b);
    assert_eq!(frontier.metrics.snapshot().urls, 1002);
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

#[test]
fn a_deferred_origin_does_not_block_a_ready_origin_of_the_same_site() {
    let directory = tempfile::tempdir().unwrap();
    let frontier =
        Frontier::open_database(directory.path().join("frontier.json"), 256, 10000).unwrap();
    let now = chrono::Utc::now();
    frontier
        .admit_urls((0..2000).map(|i| Url::parse(&format!("https://a.example.com/{i}")).unwrap()))
        .unwrap();
    let other = Url::parse("https://z.example.com/ready").unwrap();
    frontier.admit_urls([other.clone()]).unwrap();
    let first = frontier.claim_turn_at(now).unwrap().unwrap();
    let id = frontier.begin_url(&first.urls[0].url).unwrap().unwrap();
    let mut outcome = JobOutcome::default();
    outcome.record(
        first.urls[0].url.clone(),
        FetchOutcome::Deferred {
            reason: "slow_origin".into(),
            retry_after_ms: Some(3600000),
        },
    );
    frontier
        .resolve_turn(&first.domain, &outcome, 10, now)
        .unwrap();
    let next = frontier.claim_turn_at(now).unwrap().unwrap();
    assert_eq!(next.domain, first.domain);
    assert_eq!(next.urls[0].url, other);
    let mut outcome = JobOutcome::default();
    outcome.record(
        other,
        FetchOutcome::Skipped {
            reason: "fixture_done".into(),
        },
    );
    frontier
        .resolve_turn(&next.domain, &outcome, 10, now)
        .unwrap();
    assert!(frontier.claim_turn_at(now).unwrap().is_none());
    assert_eq!(
        frontier.next_eligible_at().unwrap(),
        Some(millis(now) + 3600000)
    );
    drop(frontier);
    let restored =
        Frontier::open_database(directory.path().join("frontier.json"), 256, 10000).unwrap();
    assert!(restored.claim_turn_at(now).unwrap().is_none());
    assert_eq!(restored.begin_url(&first.urls[0].url).unwrap().unwrap(), id);
}
