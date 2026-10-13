use astiango::{
    config::{LocalConfig, WarcSource},
    warc::{consumer::Consumer, stream::Limits},
    Result,
};
use serde_json::json;
use std::{
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    time::Instant,
};

fn source(directory: &Path, name: &str) -> WarcSource {
    WarcSource::Local(LocalConfig {
        folder: directory.to_string_lossy().into_owned(),
        names: vec![name.into()],
    })
}

fn fixture(directory: &Path) -> Result<()> {
    fs::create_dir_all(directory)?;
    let corpus = fs::read_to_string("crates/core/testcases/entity/world_heritage_site.html")?;
    let mut normal = File::create(directory.join("capture.warc.gz"))?;
    for i in 0..24 {
        let url = url::Url::parse(&format!("https://example.test/page/{i}?sig=keep%2Fexact"))?;
        let headers = if i == 0 {
            vec![astiango::capture::Header {
                name: "x-robots-tag".into(),
                value_hex: astiango::capture::hex(b"noindex"),
            }]
        } else {
            vec![]
        };
        let capture = astiango::capture::fixture(
            &url,
            uuid::Uuid::new_v4().to_string(),
            corpus.as_bytes().to_vec(),
            headers,
        );
        astiango::warc::capture::write(
            &mut normal,
            &capture,
            astiango::warc::PayloadType::Html,
            3,
            true,
        )?;
    }
    normal.sync_all()?;
    let mut big = File::create(directory.join("large.warc.gz"))?;
    let mut seed = 1u64;
    for i in 0..32 {
        let mut adversarial = String::with_capacity(2 * 1024 * 1024);
        for _ in 0..2 * 1024 * 1024 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            adversarial.push((b'A' + (seed % 26) as u8) as char);
        }
        let html = format!("{corpus}<script type=application/json>{adversarial}</script>");
        let url = url::Url::parse(&format!("https://example.test/large/{i}"))?;
        let capture = astiango::capture::fixture(
            &url,
            uuid::Uuid::new_v4().to_string(),
            html.into_bytes(),
            vec![],
        );
        astiango::warc::capture::write(
            &mut big,
            &capture,
            astiango::warc::PayloadType::Html,
            1,
            true,
        )?;
    }
    big.sync_all()?;
    println!(
        "{}",
        json!({"created":true,"records":24,"large_records":32,"compressed_bytes":fs::metadata(directory.join("large.warc.gz"))?.len()})
    );
    Ok(())
}

fn scan(directory: &Path, mode: &str) -> Result<()> {
    let started = Instant::now();
    let mut records = 0;
    let mut payload_bytes = 0;
    if mode == "legacy" {
        let archive = astiango::warc::WarcFile::open(directory.join("large.warc.gz"))?;
        for record in archive.records() {
            let record = record?;
            records += 1;
            payload_bytes += record.response.body.len();
        }
    } else {
        for record in astiango::warc::stream::open(
            &source(directory, "large.warc.gz"),
            "large.warc.gz",
            Limits::default(),
        )? {
            let record = record?;
            records += 1;
            payload_bytes += record.response.body.len();
        }
    }
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
    unsafe {
        libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr());
    }
    let usage = unsafe { usage.assume_init() };
    let cpu_ms = (usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) as f64 * 1000.0
        + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as f64 / 1000.0;
    anyhow::ensure!(records == 32, "large segment lost records");
    println!(
        "{}",
        json!({"mode":mode,"records":records,"payload_bytes":payload_bytes,"elapsed_ms":started.elapsed().as_secs_f64()*1000.0,"cpu_ms":cpu_ms,"peak_rss_bytes":usage.ru_maxrss*1024})
    );
    Ok(())
}

fn consume(directory: &Path, crash: bool) -> Result<()> {
    let path = directory.join("atomic-index");
    let mut index = astiango::index::Index::open(&path)?;
    index
        .inverted_index
        .prepare_writer_with_budget(32 * 1024 * 1024)?;
    let mut consumer = Consumer::open(
        &source(directory, "capture.warc.gz"),
        "capture.warc.gz",
        &directory.join("atomic-checkpoints"),
        Limits::default(),
    )?;
    consumer.recover_commit(index.inverted_index.commit_payload()?.as_deref())?;
    while let Some(batch) = consumer.next_batch(1)? {
        for record in batch.records {
            if !record.indexing_allowed() {
                continue;
            }
            let page = astiango::webpage::Webpage::from(astiango::webpage::Html::parse(
                &record.response.body,
                &record.request.url,
            )?);
            index.insert(&page)?;
        }
        index.commit_with_payload(&consumer.payload(batch.next_record, false, vec![])?)?;
        if crash && batch.next_record >= 2 {
            let mut marker = File::create(directory.join("committed.marker"))?;
            marker.write_all(b"durable engine commit before consumer confirmation")?;
            marker.sync_all()?;
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
        consumer.confirm(batch.next_record, false)?;
    }
    index.commit_with_payload(&consumer.payload(consumer.cursor(), true, vec![])?)?;
    consumer.finish()?;
    let count = index.inverted_index.num_documents();
    anyhow::ensure!(
        count == 23,
        "idempotent consumer produced {count} documents; expected 23"
    );
    println!(
        "{}",
        json!({"durable_documents":count,"records":consumer.cursor(),"noindex_archived":true})
    );
    Ok(())
}

fn integrations(directory: &Path) -> Result<()> {
    let src = source(directory, "capture.warc.gz");
    let index_config: astiango::config::IndexerConfig = serde_json::from_value(json!({
        "output_path":directory.join("published-index"),"warc_source":src,
        "host_centrality_store_path":directory.join("centrality"),"batch_size":4,
        "autocommit_after_num_inserts":8,"consumer":{"workers":2,"batch_bytes":33554432}
    }))?;
    for _ in 0..2 {
        astiango::entrypoint::indexer::run(&index_config)?;
        let index = astiango::index::Index::open(&index_config.output_path)?;
        anyhow::ensure!(
            index.inverted_index.num_documents() == 23,
            "indexer CLI repeated manifest or lost a document"
        );
    }
    let body = fs::read("crates/core/testcases/entity/world_heritage_site.html")?;
    let url = url::Url::parse("https://secondary.example.test/page")?;
    let capture = astiango::capture::fixture(&url, uuid::Uuid::new_v4().to_string(), body, vec![]);
    let mut archive = File::create(directory.join("second.warc.gz"))?;
    astiango::warc::capture::write(
        &mut archive,
        &capture,
        astiango::warc::PayloadType::Html,
        3,
        true,
    )?;
    archive.sync_all()?;
    let mut multi = index_config.clone();
    multi.output_path = directory
        .join("multiple-input-index")
        .to_string_lossy()
        .into_owned();
    if let WarcSource::Local(source) = &mut multi.warc_source {
        source.names.push("second.warc.gz".into());
    }
    for _ in 0..2 {
        astiango::entrypoint::indexer::run(&multi)?;
        anyhow::ensure!(
            astiango::index::Index::open(&multi.output_path)?
                .inverted_index
                .num_documents()
                == 24,
            "multiple input publication lost or duplicated documents"
        );
    }
    let graph_config: astiango::config::WebgraphConstructConfig = serde_json::from_value(json!({
        "graph_base_path":directory.join("published-graph"),"shard":0,"warc_source":source(directory,"capture.warc.gz"),
        "host_centrality_store_path":directory.join("graph-centrality"),"host_rank_store_path":directory.join("graph-rank"),
        "consumer":{"workers":2,"batch_bytes":33554432}
    }))?;
    astiango::entrypoint::Webgraph::run(&graph_config)?;
    let before =
        astiango::webgraph::Webgraph::open(&graph_config.graph_base_path, graph_config.shard)?
            .page_edges()
            .count();
    anyhow::ensure!(before > 0, "webgraph did not consume archived links");
    astiango::entrypoint::Webgraph::run(&graph_config)?;
    let after =
        astiango::webgraph::Webgraph::open(&graph_config.graph_base_path, graph_config.shard)?
            .page_edges()
            .count();
    anyhow::ensure!(
        before == after,
        "repeated manifest duplicated logical graph edges"
    );
    let mut multi_graph = graph_config.clone();
    multi_graph.graph_base_path = directory
        .join("multiple-input-graph")
        .to_string_lossy()
        .into_owned();
    multi_graph.warc_source = multi.warc_source.clone();
    astiango::entrypoint::Webgraph::run(&multi_graph)?;
    let multi_edges =
        astiango::webgraph::Webgraph::open(&multi_graph.graph_base_path, multi_graph.shard)?
            .page_edges()
            .count();
    astiango::entrypoint::Webgraph::run(&multi_graph)?;
    anyhow::ensure!(
        multi_edges > before
            && multi_edges
                == astiango::webgraph::Webgraph::open(
                    &multi_graph.graph_base_path,
                    multi_graph.shard
                )?
                .page_edges()
                .count(),
        "multiple graph inputs changed logical edges on replay"
    );
    let corrupt = directory.join("corrupt.warc.gz");
    fs::copy(directory.join("capture.warc.gz"), &corrupt)?;
    let file = fs::OpenOptions::new().write(true).open(&corrupt)?;
    file.set_len(file.metadata()?.len() - 37)?;
    let mut consumer = Consumer::open(
        &source(directory, "corrupt.warc.gz"),
        "corrupt.warc.gz",
        &directory.join("corrupt-checkpoints"),
        Limits::default(),
    )?;
    let mut failed = false;
    loop {
        match consumer.next_batch(1) {
            Ok(Some(batch)) => consumer.confirm(batch.next_record, false)?,
            Ok(None) => break,
            Err(_) => {
                failed = true;
                break;
            }
        }
    }
    anyhow::ensure!(failed, "corrupt segment was silently accepted");
    drop(consumer);
    anyhow::ensure!(
        Consumer::open(
            &source(directory, "corrupt.warc.gz"),
            "corrupt.warc.gz",
            &directory.join("corrupt-checkpoints"),
            Limits::default()
        )
        .is_err(),
        "quarantine did not survive restart"
    );
    let mut limits = Limits::default();
    limits.record_bytes = 4096;
    let mut limited = astiango::warc::stream::open(
        &source(directory, "capture.warc.gz"),
        "capture.warc.gz",
        limits,
    )?;
    anyhow::ensure!(limited.next().unwrap().is_err(), "record quota ignored");
    let url = url::Url::parse("https://metadata-budget.test/page")?;
    let mut capture = astiango::capture::fixture(
        &url,
        uuid::Uuid::new_v4().to_string(),
        fs::read("crates/core/testcases/entity/world_heritage_site.html")?,
        vec![],
    );
    for i in 0..512 {
        capture
            .metadata
            .discovery_urls
            .push(url::Url::parse(&format!(
                "https://metadata-budget.test/{i}?sig={}",
                "x".repeat(1024)
            ))?);
    }
    let mut archive = File::create(directory.join("metadata.warc.gz"))?;
    astiango::warc::capture::write(
        &mut archive,
        &capture,
        astiango::warc::PayloadType::Html,
        3,
        true,
    )?;
    archive.sync_all()?;
    let mut limits = Limits::default();
    limits.decoded_bytes = 256 * 1024;
    limits.batch_bytes = 1024 * 1024;
    let mut metadata_limited = Consumer::open(
        &source(directory, "metadata.warc.gz"),
        "metadata.warc.gz",
        &directory.join("metadata-checkpoint"),
        limits,
    )?;
    anyhow::ensure!(
        metadata_limited.next_batch(64).is_err(),
        "metadata ignored the consumer byte budget"
    );
    let quota = astiango::crawler::resources::cpu_quota("200000 100000");
    anyhow::ensure!(quota == Some(2), "cgroup quota misread");
    let config = astiango::crawler::resources::Config {
        automatic: true,
        memory_limit_bytes: 512 * 1024 * 1024,
        daily_network_bytes: 1000,
        ..Default::default()
    };
    let controller =
        astiango::crawler::resources::Controller::new(config.clone(), directory.join("quota"), 2)?;
    controller.sample(1000, 0, 1073741824)?;
    anyhow::ensure!(
        controller.stats().workers_allowed == 0 && controller.stats().reason == "network_budget",
        "quota did not stop admission"
    );
    drop(controller);
    let restored =
        astiango::crawler::resources::Controller::new(config, directory.join("quota"), 2)?;
    restored.sample(0, 0, 1073741824)?;
    anyhow::ensure!(
        restored.stats().network_bytes_today == 1000 && restored.stats().workers_allowed == 0,
        "quota reset after restart"
    );
    println!(
        "{}",
        json!({"passed":true,"indexer_manifests_repeated":2,"indexed_documents":23,"multiple_manifest_documents":24,"webgraph_manifests_repeated":2,"corruption_quarantined":true,"record_limit":true,"metadata_byte_budget":true,"cgroup_cpu_quota":2,"network_quota_restart":true})
    );
    Ok(())
}

fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let action = args.get(1).map(String::as_str).unwrap_or("integrations");
    let directory = PathBuf::from(
        args.get(2)
            .map(String::as_str)
            .unwrap_or("data/crawler-operations-bench"),
    );
    match action {
        "create" => fixture(&directory),
        "scan" => scan(
            &directory,
            args.get(3).map(String::as_str).unwrap_or("stream"),
        ),
        "crash-commit" => consume(&directory, true),
        "resume" => consume(&directory, false),
        "integrations" => integrations(&directory),
        _ => anyhow::bail!("unknown operation"),
    }
}
