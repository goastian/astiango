//! Frontier/control/restart integration. No HTTP is sent to fixture domains.
use anyhow::{Context, Result};
use astiango_crawler_service::crawler::{frontier::Frontier, Job, JobOutcome};
use chrono::{Duration, Utc};
use serde_json::json;
use std::{fs, path::PathBuf};
use url::Url;

fn settle(frontier: &Frontier, job: &Job, now: chrono::DateTime<Utc>, defer: bool) -> Result<()> {
    let outcome: JobOutcome = serde_json::from_value(json!({
        "status":if defer {"deferred"} else {"completed"},
        "counters":{"page_http_attempts":0,"redirect_hops":0,"useful_pages":0},
        "urls":[{"url":job.urls.front().unwrap().url,
            "outcome":if defer {json!({"kind":"deferred","reason":"origin_not_eligible","retry_after_ms":120000})}
                else {json!({"kind":"skipped","reason":"fixture_complete"})}}]
    }))?;
    frontier.resolve_turn(&job.domain, &outcome, 10000, now)
}

fn submit(frontier: &Frontier, controls: &std::path::Path, urls: &[Url]) -> Result<serde_json::Value> {
    let id = uuid::Uuid::new_v4().to_string();
    let request = json!({"id":id,"action":"enqueue_urls","urls":urls,"priority":true});
    fs::write(controls.join(format!("{id}.request")), request.to_string())?;
    frontier.process_controls(controls)?;
    Ok(serde_json::from_slice(&fs::read(controls.join(format!("{id}.result")))?)?)
}

fn reserve_and_recovery(root: &std::path::Path) -> Result<()> {
    let controls=root.join("reserve-control"); fs::create_dir_all(&controls)?;
    let frontier=Frontier::open_database(root.join("reserve.redb"),16,1)?;
    frontier.configure_manual_reserve(1)?;
    let seed=Url::parse("https://quota.test/seed")?;
    frontier.discover([seed])?;
    let now=Utc::now();
    let job=frontier.claim_turn_at(now)?.context("missing initial quota job")?;
    settle(&frontier,&job,now,false)?;
    let homepage=Url::parse("https://quota.test/")?;
    let over=Url::parse("https://quota.test/over-reserve")?;
    let receipt=submit(&frontier,&controls,&[homepage.clone(),over])?;
    anyhow::ensure!(receipt["admitted"]==1 && receipt["decisions"][1]["reason"]=="site_admission_quota", "manual reserve did not enforce its limit: {receipt}");
    let job=frontier.claim_turn_at(now+Duration::minutes(3))?.context("manual URL did not reopen finished site")?;
    anyhow::ensure!(job.urls.front().unwrap().url==homepage,"wrong reopened URL");
    let outcome:JobOutcome=serde_json::from_value(json!({"status":"completed",
        "counters":{"page_http_attempts":0,"redirect_hops":0,"useful_pages":0},
        "urls":[{"url":homepage,"outcome":{"kind":"skipped","reason":"outside_job_domain"}}]}))?;
    frontier.resolve_turn(&job.domain,&outcome,10000,now+Duration::minutes(3))?;
    let receipt=submit(&frontier,&controls,&[homepage.clone()])?;
    anyhow::ensure!(receipt["decisions"][0]["recovered"]==true,"known redirect scope failure was not recovered: {receipt}");
    let job=frontier.claim_turn_at(now+Duration::minutes(6))?.context("recovered redirect missing")?;
    anyhow::ensure!(job.urls.front().unwrap().url==homepage,"wrong recovered URL");
    let blocked:JobOutcome=serde_json::from_value(json!({"status":"completed",
        "counters":{"page_http_attempts":0,"redirect_hops":0,"useful_pages":0},
        "urls":[{"url":homepage,"outcome":{"kind":"skipped","reason":"robots_disallowed"}}]}))?;
    frontier.resolve_turn(&job.domain,&blocked,10000,now+Duration::minutes(6))?;
    let receipt=submit(&frontier,&controls,&[homepage])?;
    anyhow::ensure!(receipt["decisions"][0]["recovered"]==false && receipt["decisions"][0]["prioritized"]==false,"robots exclusion was reset");
    anyhow::ensure!(frontier.metrics.snapshot().urls==2,"reserve or recovery changed URL identity/counts");
    Ok(())
}

fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args().nth(1).context("provide a new fixture directory")?);
    fs::create_dir_all(&root)?;
    let path = root.join("frontier.redb");
    anyhow::ensure!(!path.exists(), "fixture database must be new");
    let controls = root.join("control"); fs::create_dir_all(&controls)?;
    let normal = (0..64).map(|i| Url::parse(&format!("https://discovery-{i}.test/"))).collect::<Result<Vec<_>,_>>()?;
    let priority = (0..4).map(|i| Url::parse(&format!("https://operator-{i}.test/?sig=Keep%2F&x=1&x=2"))).collect::<Result<Vec<_>,_>>()?;
    let frontier = Frontier::open_database(path.clone(), 256, 10)?;
    frontier.discover(normal)?;
    let id = uuid::Uuid::new_v4().to_string();
    let request = json!({"id":id,"action":"enqueue_urls","urls":priority,"priority":true});
    fs::write(controls.join("manual-actions.jsonl"), format!("{request}\n"))?;
    fs::write(controls.join(format!("{id}.request")), request.to_string())?;
    frontier.process_controls(&controls)?;
    let receipt: serde_json::Value = serde_json::from_slice(&fs::read(controls.join(format!("{id}.result")))?)?;
    anyhow::ensure!(receipt["admitted"] == 4 && receipt["decisions"].as_array().unwrap().iter().all(|r|r["prioritized"]==true), "priority admission not confirmed");
    drop(frontier);
    let frontier = Frontier::open_database(path, 256, 10)?;
    let now = Utc::now();
    for (i, url) in priority.iter().enumerate().take(3) {
        let job = frontier.claim_turn_at(now)?.context("missing priority job")?;
        anyhow::ensure!(job.urls.front().unwrap().url == *url, "operator URL was not selected before discovery");
        settle(&frontier, &job, now, i == 0)?;
    }
    let regular = frontier.claim_turn_at(now)?.context("missing regular turn")?;
    anyhow::ensure!(regular.urls.front().unwrap().url.host_str().unwrap().starts_with("discovery-"), "discovery starved after three priority turns");
    settle(&frontier, &regular, now, false)?;
    let fourth = frontier.claim_turn_at(now)?.context("missing fourth priority")?;
    anyhow::ensure!(fourth.urls.front().unwrap().url == priority[3], "fourth priority was lost");
    settle(&frontier, &fourth, now, false)?;
    let early = frontier.claim_turn_at(now + Duration::seconds(119))?.context("missing eligible discovery turn")?;
    anyhow::ensure!(early.urls.front().unwrap().url != priority[0], "priority shortened remote delay");
    settle(&frontier, &early, now + Duration::seconds(119), false)?;
    let due = frontier.claim_turn_at(now + Duration::seconds(121))?.context("missing deferred priority")?;
    anyhow::ensure!(due.urls.front().unwrap().url == priority[0], "deferred priority did not resume");
    anyhow::ensure!(frontier.metrics.snapshot().urls == 68, "prioritization changed admitted coverage");
    reserve_and_recovery(&root)?;
    println!("{}",json!({"passed":true,"http_requests":0,"urls":68,"priority_survives_restart":true,
        "regular_turn_reserved":true,"delay_preserved":true,"signed_query_preserved":true,
        "bounded_manual_reserve":true,"completed_site_reopened":true,"scope_failure_recovered":true,"robots_exclusion_preserved":true}));
    Ok(())
}
