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
                weight: 1.0,
                url: url.into(),
                origin: Url::parse(url).unwrap().origin().ascii_serialization(),
                site: "github.io".into(),
                round: 1,
                state,
                fetch_id: id.map(str::to_owned),
                reason: None,
                not_before: 0,
                sitemap: false,
                retry: Default::default(),
                sitemap_depth: 0,
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
fn recrawl_from_an_older_round_preserves_payload_and_reserves_a_new_fetch() {
    let directory = tempfile::tempdir().unwrap();
    let frontier = open(directory.path());
    frontier
        .configure_recrawl(&super::super::recrawl::Config {
            min_interval_ms: 1,
            initial_interval_ms: 1,
            max_interval_ms: 1,
            ..Default::default()
        })
        .unwrap();
    let url: Url = "https://example.org/unchanged?signature=keep"
        .parse()
        .unwrap();
    frontier.admit_urls([url.clone()]).unwrap();
    let job = frontier.claim_turn_at(chrono::Utc::now()).unwrap().unwrap();
    let id = frontier.begin_url(&url).unwrap().unwrap();
    let capture = crate::capture::fixture(&url, id.clone(), b"body".to_vec(), Vec::new());
    frontier
        .commit_capture(&capture.metadata, &receipt(&capture), false)
        .unwrap();
    frontier.drain_outbox().unwrap();
    let mut outcome = JobOutcome::default();
    outcome.record(
        url.clone(),
        FetchOutcome::DurableSuccess {
            final_url: url.clone(),
        },
    );
    frontier
        .resolve_turn(&job.domain, &outcome, 0, chrono::Utc::now())
        .unwrap();
    assert!(frontier.is_empty());
    frontier.restart_round(Vec::new()).unwrap();
    drop(frontier);
    let frontier = open(directory.path());
    let now = chrono::Utc::now() + chrono::Duration::seconds(1);
    assert_eq!(frontier.requeue_recrawls(now).unwrap(), 1);
    let job = frontier.claim_turn_at(now).unwrap().unwrap();
    assert_eq!(job.urls[0].url, url);
    assert_ne!(frontier.begin_url(&url).unwrap().unwrap(), id);
    assert_eq!(
        frontier
            .revalidation_state(&url)
            .unwrap()
            .unwrap()
            .reference
            .fetch_id,
        id
    );
    assert_eq!(
        frontier
            .revalidation_state(&url)
            .unwrap()
            .unwrap()
            .reference
            .record_offset,
        100
    );
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

#[test]
fn retry_after_is_durable_and_exhaustion_is_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let now = chrono::Utc::now();
    let url: Url = "https://retry.example.org/page".parse().unwrap();
    let domain = Domain::from(&url);
    let frontier = open(dir.path());
    frontier.discover([url.clone()]).unwrap();
    frontier.claim_turn_at(now).unwrap().unwrap();
    frontier.begin_url(&url).unwrap().unwrap();
    frontier
        .authorize_request(&url, &url, now)
        .unwrap()
        .unwrap();
    let mut outcome = JobOutcome::default();
    outcome.record(
        url.clone(),
        FetchOutcome::Deferred {
            reason: "http_503".into(),
            retry_after_ms: Some(3_600_000),
        },
    );
    frontier.resolve_turn(&domain, &outcome, 10, now).unwrap();
    drop(frontier);
    let frontier = open(dir.path());
    assert!(frontier
        .claim_turn_at(now + chrono::Duration::minutes(59))
        .unwrap()
        .is_none());
    assert!(frontier
        .claim_turn_at(now + chrono::Duration::hours(1))
        .unwrap()
        .is_some());
    assert_eq!(frontier.metrics.snapshot().retry_deferred, 1);
    let mut config = super::super::retry::Config::default();
    config.max_attempts = 2;
    frontier.configure_retry(&config).unwrap();
    frontier.begin_url(&url).unwrap().unwrap();
    frontier
        .resolve_turn(&domain, &outcome, 10, now + chrono::Duration::hours(1))
        .unwrap();
    let tx = frontier.db.begin_read().unwrap();
    let table = tx.open_table(URLS).unwrap();
    let row: UrlRow =
        serde_json::from_slice(table.get(url.as_str()).unwrap().unwrap().value()).unwrap();
    assert_eq!(row.state, State::Terminal);
    assert!(row.reason.unwrap().starts_with("retry_budget_exhausted"));
    assert_eq!(frontier.metrics.snapshot().retry_exhausted, 1);
    assert!(frontier.is_empty());
}

#[test]
fn not_found_is_slow_gone_is_retired_and_status_digits_are_not_guessed() {
    let dir = tempfile::tempdir().unwrap();
    let frontier = open(dir.path());
    let now = chrono::Utc::now();
    let urls: Vec<Url> = ["404", "410", "certificate410"]
        .into_iter()
        .map(|path| format!("https://example.org/{path}").parse().unwrap())
        .collect();
    frontier.admit_urls(urls.clone()).unwrap();
    while let Some(job) = frontier.claim_turn_at(now).unwrap() {
        let url = job.urls[0].url.clone();
        frontier.begin_url(&url).unwrap().unwrap();
        let reason = match url.path() {
            "/404" => "http_404",
            "/410" => "http_410",
            _ => "certificate validation failed for host410",
        };
        let mut outcome = JobOutcome::default();
        outcome.record(
            url,
            FetchOutcome::PermanentFailure {
                reason: reason.into(),
            },
        );
        frontier
            .resolve_turn(&job.domain, &outcome, 0, now)
            .unwrap();
    }
    assert!(frontier.is_empty());
    frontier.restart_round(Vec::new()).unwrap();
    frontier.admit_urls(urls.clone()).unwrap();
    let tx = frontier.db.begin_read().unwrap();
    let table = tx.open_table(URLS).unwrap();
    let row = |index: usize| {
        serde_json::from_slice::<UrlRow>(table.get(urls[index].as_str()).unwrap().unwrap().value())
            .unwrap()
    };
    let missing = row(0);
    let gone = row(1);
    let other = row(2);
    assert!(missing.not_before >= millis(now) + 604_800_000);
    assert_eq!((gone.state, gone.round), (State::Terminal, 1));
    assert_eq!((other.state, other.round), (State::Pending, 2));
}

#[test]
fn incremental_sitemap_feed_quotas_malformed_xml_and_reversible_traps() {
    let dir = tempfile::tempdir().unwrap();
    let frontier = Frontier::open_database(dir.path().join("frontier.redb"), 128, 50).unwrap();
    let source: Url = "https://example.org/sitemap.xml".parse().unwrap();
    let mut xml = String::from("<urlset>");
    for i in 0..50_000 {
        xml.push_str(&format!("<url><loc>https://example.org/article/{i}?signed={i}</loc><lastmod>2026-10-10</lastmod></url>"));
    }
    xml.push_str("</urlset>");
    assert_eq!(
        frontier.ingest_sitemap(&source, &xml).unwrap().candidates,
        50_000
    );
    assert_eq!(frontier.candidate_count().unwrap(), 50_000);
    assert_eq!(frontier.metrics.snapshot().urls, 50);
    assert_eq!(
        frontier.ingest_sitemap(&source, &xml).unwrap().reason,
        "already_processed"
    );
    assert_eq!(frontier.metrics.snapshot().quarantined_urls, 0);
    assert_eq!(
        frontier
            .ingest_sitemap(
                &"https://example.org/bad.xml".parse().unwrap(),
                "<urlset><url><loc>&broken;</loc>"
            )
            .unwrap()
            .reason,
        "invalid_discovery_xml"
    );
    let feed =
        "<rss><channel><item><link>https://example.org/news/new</link></item></channel></rss>";
    let feed_url = "https://example.org/feed.xml".parse().unwrap();
    frontier
        .ingest_feed(&feed_url, feed, crate::feed::FeedKind::Rss)
        .unwrap();
    let count = frontier.candidate_count().unwrap();
    frontier
        .ingest_feed(&feed_url, feed, crate::feed::FeedKind::Rss)
        .unwrap();
    assert_eq!(frontier.candidate_count().unwrap(), count);
    let traps = super::super::traps::Config {
        calendar_family_budget: 3,
        ..Default::default()
    };
    frontier
        .configure_discovery(&Default::default(), &traps)
        .unwrap();
    for day in 1..=10 {
        frontier
            .admit_urls([format!("https://calendar.example/calendar/2026/10/{day}")
                .parse()
                .unwrap()])
            .unwrap();
    }
    assert_eq!(frontier.metrics.snapshot().quarantined_urls, 7);
    let url: Url = "https://calendar.example/calendar/2026/10/10"
        .parse()
        .unwrap();
    frontier.release_quarantine(&url).unwrap();
    assert_eq!(frontier.metrics.snapshot().quarantine_released, 1);
    let tx = frontier.db.begin_read().unwrap();
    assert!(tx
        .open_table(URLS)
        .unwrap()
        .get(url.as_str())
        .unwrap()
        .is_some());
}

#[test]
fn courtesy_is_not_a_failed_http_attempt_and_invalid_control_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let frontier = open(dir.path());
    let url: Url = "https://example.org/a".parse().unwrap();
    frontier.discover([url.clone()]).unwrap();
    let now = chrono::Utc::now();
    frontier.claim_turn_at(now).unwrap().unwrap();
    let mut outcome = JobOutcome::default();
    outcome.record(
        url.clone(),
        FetchOutcome::Deferred {
            reason: "origin_not_eligible".into(),
            retry_after_ms: Some(10),
        },
    );
    frontier
        .resolve_turn(&Domain::from(&url), &outcome, 10, now)
        .unwrap();
    assert_eq!(frontier.metrics.snapshot().retry_exhausted, 0);
    assert_eq!(frontier.metrics.snapshot().terminal_urls, 0);
    let control = dir.path().join("control");
    fs::create_dir(&control).unwrap();
    for index in 0..200 {
        fs::write(control.join(format!("{index}.result")), b"{}").unwrap();
    }
    fs::write(control.join("bad.request"), b"broken json").unwrap();
    frontier.process_controls(&control).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&fs::read(control.join("bad.result")).unwrap())
            .unwrap()["state"],
        "rejected"
    );
}

#[test]
fn discovery_quota_sources_cycles_and_promotion_do_not_starve_other_tenants() {
    let dir = tempfile::tempdir().unwrap();
    let frontier = Frontier::open_database(dir.path().join("frontier.redb"), 128, 2).unwrap();
    let config = super::super::discovery::Config {
        max_urls: 2000,
        ..Default::default()
    };
    frontier
        .configure_discovery(&config, &Default::default())
        .unwrap();
    let source: Url = "https://aaa.example/sitemap.xml".parse().unwrap();
    let mut xml = String::from("<urlset>");
    for i in 0..2001 {
        xml.push_str(&format!("<url><loc>https://aaa.example/{i:06}</loc></url>"));
    }
    xml.push_str("</urlset>");
    assert_eq!(
        frontier.ingest_sitemap(&source, &xml).unwrap().reason,
        "discovery_aggregate_quota"
    );
    assert_eq!(frontier.candidate_urls(&source, 512).unwrap().len(), 512);
    let other: Url = "https://zzz.example/sitemap.xml".parse().unwrap();
    frontier
        .ingest_sitemap(
            &other,
            "<urlset><url><loc>https://zzz.example/ready</loc></url></urlset>",
        )
        .unwrap();
    frontier.promote_candidates().unwrap();
    assert_eq!(frontier.metrics.snapshot().known_sites, 2);
    let cycle: Url = "https://cycle.example/index.xml".parse().unwrap();
    let xml="<sitemapindex><sitemap><loc>https://cycle.example/index.xml</loc></sitemap></sitemapindex>";
    assert_eq!(frontier.ingest_sitemap(&cycle, xml).unwrap().candidates, 1);
    assert_eq!(
        frontier.ingest_sitemap(&cycle, xml).unwrap().reason,
        "already_processed"
    );
}
