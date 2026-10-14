//! Streaming consumer of specialized derivatives. Captures remain the authority;
//! a derivative is never represented as a fabricated HTTP response.
use std::{fs::{File, OpenOptions}, io::{BufRead, BufReader}, path::Path};
use anyhow::{Context, Result};
use fs4::FileExt;
use serde::{Deserialize, Serialize};
use crate::{capture, crawler::dedup::{Config, ExactSet}, index::Index, webpage::{Html, Webpage}};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Document {
    pub(crate) version: u32,
    pub(crate) id: String,
    pub(crate) source: capture::Metadata,
    pub(crate) kind: String,
    pub(crate) derived_sha256: String,
    pub(crate) text: String,
    pub(crate) title: String,
    pub(crate) indexing_allowed: bool,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Checkpoint { kind: String, version: u32, keys: Vec<String> }
#[derive(Default, Serialize)]
pub struct Report { pub indexed: u64, pub duplicates: u64, pub noindex: u64, pub documents: u64 }
fn escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
pub fn run(input: &Path, output: &Path) -> Result<Report> {
    std::fs::create_dir_all(output)?;
    let lock = OpenOptions::new().create(true).truncate(false).write(true).open(output.join("derived.lock"))?;
    lock.try_lock_exclusive().context("derived index consumer is already running")?;
    let mut index = Index::open(output)?;
    index.inverted_index.prepare_writer_with_budget(64 * 1024 * 1024)?;
    let mut exact = ExactSet::open(&output.join("derived-exact.redb"), &Config::default())?;
    if let Some(payload) = index.inverted_index.commit_payload()? {
        let checkpoint: Checkpoint = serde_json::from_str(&payload).context("output belongs to another index consumer")?;
        anyhow::ensure!(checkpoint.kind == "derived-index" && checkpoint.version == 1 && checkpoint.keys.len() <= 32, "unknown derived index checkpoint version");
        exact.commit(&checkpoint.keys)?;
    }
    let mut reader = BufReader::with_capacity(65536, File::open(input)?);
    let mut report = Report::default();
    let mut line = Vec::new();
    loop {
        line.clear();
        // read_until alone would allocate an unbounded malicious JSON line.
        while !reader.fill_buf()?.is_empty() {
            let buffer = reader.fill_buf()?;
            let amount = buffer.iter().position(|b| *b == b'\n').map_or(buffer.len(), |n| n + 1);
            anyhow::ensure!(line.len() + amount <= 16 * 1024 * 1024, "derived document exceeds record budget");
            line.extend_from_slice(&buffer[..amount]); reader.consume(amount);
            if line.last() == Some(&b'\n') { break; }
        }
        if line.is_empty() { break; }
        let document: Document = serde_json::from_slice(&line).with_context(|| format!("invalid derived record {}", report.documents + 1))?;
        anyhow::ensure!(document.version == 1, "unknown derived document version {}", document.version);
        anyhow::ensure!(matches!(document.kind.as_str(), "pdf-text-v1" | "offline-js-text-v1"), "unsupported specialized derivation {}", document.kind);
        document.source.validate()?;
        anyhow::ensure!(document.text.len() <= 2 * 1024 * 1024 && document.title.len() <= 8192 && capture::sha256(document.text.as_bytes()) == document.derived_sha256, "derived digest or byte budget mismatch");
        anyhow::ensure!(document.id == format!("{}:{}", document.source.fetch_id, document.kind), "derived identity mismatch");
        let key = format!("{}:{}", document.id, document.derived_sha256);
        report.documents += 1;
        if exact.contains(&key)? { report.duplicates += 1; continue; }
        // Original noindex is irreversible by a derivative. No download is
        // prohibited merely because indexing is prohibited.
        if !document.indexing_allowed || document.source.robots.no_index { report.noindex += 1; continue; }
        let markup = format!("<html><head><title>{}</title></head><body><article>{}</article></body></html>", escape(&document.title), escape(&document.text));
        let mut html = Html::parse(&markup, &document.source.response.url)?;
        html.set_clean_text(document.text);
        let mut webpage = Webpage::from(html);
        webpage.fetch_time_ms = document.source.fetch_time_ms;
        index.insert(&webpage)?;
        // The engine commit carries its exact key. Recovery closes the crash
        // window between index durability and the exact-set commit.
        let checkpoint = Checkpoint { kind: "derived-index".into(), version: 1, keys: vec![key.clone()] };
        index.commit_with_payload(&serde_json::to_string(&checkpoint)?)?;
        exact.commit(&[key])?;
        report.indexed += 1;
    }
    Ok(report)
}
