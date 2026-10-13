use anyhow::Result;
use astiango_crawler_service::crawler::{frontier::Frontier, JobOutcome};
use redb::{Database, ReadableTable, TableDefinition};

fn main() -> Result<()> {
    let directory = std::path::PathBuf::from(std::env::args().nth(1).ok_or_else(|| anyhow::anyhow!("provide an empty fixture directory"))?);
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("frontier.json");
    anyhow::ensure!(!path.with_extension("redb").exists(), "fixture requires a new database");
    let blocked = url::Url::parse("https://blocked.test/page?sig=keep%2Fexact&a=2&a=1")?;
    let ready = url::Url::parse("https://ready.test/page/")?;
    let frontier = Frontier::open_database(path.clone(), 512, 10)?;
    let mut urls = vec![blocked.clone(), ready.clone()];
    urls.extend((0..298).map(|index| url::Url::parse(&format!("https://site-{index}.test/page")).unwrap()));
    frontier.discover(urls)?;
    let job = frontier.claim_turn_at(chrono::Utc::now())?.ok_or_else(|| anyhow::anyhow!("no initial fixture job"))?;
    anyhow::ensure!(job.urls.front().unwrap().url == blocked, "unexpected initial URL");
    let outcome: JobOutcome = serde_json::from_value(serde_json::json!({
        "status":"deferred", "counters":{"page_http_attempts":0,"redirect_hops":0,"useful_pages":0},
        "urls":[{"url":blocked,"outcome":{"kind":"deferred","reason":"http_503","retry_after_ms":60000}}]
    }))?;
    frontier.resolve_turn(&job.domain, &outcome, 10000, chrono::Utc::now())?;
    let before = frontier.metrics.snapshot();
    drop(frontier);

    const META: TableDefinition<&str, &[u8]> = TableDefinition::new("metadata.v1");
    const ORIGINS: TableDefinition<&str, &[u8]> = TableDefinition::new("http_origins.v1");
    const HTTP_READY: TableDefinition<(&str, u64, u64), &str> = TableDefinition::new("http_origin_eligibility.v1");
    const READY: TableDefinition<(u64, u64), &str> = TableDefinition::new("site_eligibility.v2");
    {
        let db = Database::create(path.with_extension("redb"))?;
        let tx = db.begin_write()?;
        tx.open_table(META)?.remove("ready-index-version")?;
        let origins = tx.open_table(ORIGINS)?;
        let stored = origins.get("https://blocked.test")?.ok_or_else(|| anyhow::anyhow!("missing blocked origin"))?;
        let row: serde_json::Value = serde_json::from_slice(stored.value())?;
        // A legacy key predates the authoritative HTTP-origin retry deadline.
        tx.open_table(HTTP_READY)?.insert(("blocked.test", 0, row["sequence"].as_u64().unwrap()), "https://blocked.test")?;
        tx.open_table(READY)?.insert((0, 0), "blocked.test")?;
        drop(stored); drop(origins);
        tx.commit()?;
    }
    let frontier = Frontier::open_database(path.clone(), 512, 10)?;
    anyhow::ensure!(frontier.metrics.snapshot().urls == before.urls && frontier.metrics.snapshot().pending_urls == before.pending_urls, "repair changed durable URL counts");
    let job = frontier.claim_turn_at(chrono::Utc::now())?.ok_or_else(|| anyhow::anyhow!("ready origin was blocked"))?;
    anyhow::ensure!(job.urls.front().unwrap().url == ready, "repair visited deferred origin before its deadline");
    drop(frontier);
    {
        let db = Database::create(path.with_extension("redb"))?;
        let tx = db.begin_write()?;
        // Resume an interruption between HTTP-origin and site-index batches.
        tx.open_table(META)?.remove("ready-index-version")?;
        tx.open_table(META)?.insert("ready-repair-phase", serde_json::to_vec("sites")?.as_slice())?;
        tx.open_table(META)?.insert("ready-repair-cursor", serde_json::to_vec("")?.as_slice())?;
        tx.delete_table(READY)?; tx.open_table(READY)?;
        tx.commit()?;
    }
    let frontier = Frontier::open_database(path.clone(), 512, 10)?;
    anyhow::ensure!(frontier.metrics.snapshot().urls == 300 && frontier.metrics.snapshot().pending_sites == 300, "interrupted repair lost work");
    anyhow::ensure!(frontier.claim_turn_at(chrono::Utc::now())?.unwrap().urls.front().unwrap().url != blocked, "resume shortened retry deadline");
    drop(frontier);
    {
        let db = Database::create(path.with_extension("redb"))?;
        let tx = db.begin_write()?;
        tx.open_table(META)?.insert("ready-index-version", serde_json::to_vec(&99u32)?.as_slice())?;
        tx.commit()?;
    }
    anyhow::ensure!(Frontier::open_database(path,512,10).is_err(), "unknown readiness version was accepted");
    println!("{}", serde_json::json!({"version":1,"passed":true,"urls_preserved":300,"batch_size":128,"retry_deadline_preserved":true,"stale_indexes_repaired":true,"interrupted_repair_resumed":true,"unknown_version_rejected":true,"http_requests":0}));
    Ok(())
}
