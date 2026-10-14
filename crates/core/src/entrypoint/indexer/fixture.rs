use crate::warc::{Metadata, PayloadType, Request, Response, WarcRecord, WarcWriter};
use anyhow::Result;
use serde_json::json;
use std::{fs, path::Path};
fn legacy(url: &str, body: String) -> WarcRecord {
    WarcRecord {
        request: Request {
            url: url.into(),
            date: Some("2026-10-13T00:00:00Z".parse().unwrap()),
        },
        response: Response {
            body,
            payload_type: Some(PayloadType::Html),
        },
        metadata: Metadata {
            fetch_time_ms: 10,
            capture: None,
        },
    }
}
fn captured(url: &str, body: &[u8], second: i64, noindex: bool) -> crate::capture::Capture {
    let url = url::Url::parse(url).unwrap();
    let response = crate::capture::HttpResponse {
        url: url.to_string(),
        status: 200,
        version: "HTTP/1.1".into(),
        headers: vec![],
        received_at: chrono::Utc::now(),
        peer_ip: Some("127.0.0.1".parse().unwrap()),
        address_validated: true,
        network_policy: "fixture".into(),
    };
    let mut capture = crate::capture::Capture {
        metadata: crate::capture::Metadata {
            revalidation: None,
            schema_version: 1,
            identity_version: 1,
            fetch_id: uuid::Uuid::from_u128(10000 + second as u128).to_string(),
            requested_url: url.to_string(),
            response,
            redirects: vec![],
            payload_sha256: crate::capture::sha256(body),
            payload_bytes: body.len(),
            decoded_bytes: String::from_utf8_lossy(body).len(),
            charset: "UTF-8".into(),
            decoded_limit_bytes: 32 * 1024 * 1024,
            user_agent: crate::capture::DEFAULT_AGENT.into(),
            robots: Default::default(),
            transformations: vec!["controlled-fixture".into()],
            fetch_time_ms: 1,
            discovery_urls: vec![],
            internal_urls: vec![],
        },
        payload: std::sync::Arc::new(body.to_vec()),
    };
    capture.metadata.response.received_at = "2026-10-13T00:00:00Z"
        .parse::<chrono::DateTime<chrono::Utc>>()
        .unwrap()
        + chrono::Duration::seconds(second);
    capture.metadata.robots.no_index = noindex;
    if noindex {
        capture
            .metadata
            .response
            .headers
            .push(crate::capture::Header {
                name: "x-robots-tag".into(),
                value_hex: crate::capture::hex(b"AstianGOBot: noindex"),
            });
    }
    capture
}
pub fn create(path: &Path) -> Result<serde_json::Value> {
    fs::create_dir_all(path)?;
    let assets = [
        "crates/core/testcases/schema_org/recipe.html",
        "crates/core/testcases/schema_org/infinity_war.html",
        "crates/core/testcases/just_text/reddit.html",
    ];
    let mut contents = Vec::new();
    let mut sources = Vec::new();
    for asset in assets {
        if Path::new(asset).exists() {
            let bytes = fs::read(asset)?;
            sources.push(
                json!({"path":asset,"bytes":bytes.len(),"sha256":crate::capture::sha256(&bytes)}),
            );
            contents.push(String::from_utf8_lossy(&bytes).into_owned());
        }
    }
    anyhow::ensure!(contents.len() >= 2, "real saved HTML corpus missing");
    let hosts = [
        "https://www.valdemarsro.dk/",
        "https://www.businessinsider.com/",
        "https://www.reddit.com/",
    ];
    let mut graph =
        crate::webgraph::WebgraphBuilder::new(path.join("graph"), 0u64.into()).open()?;
    graph.prepare_writer_with_budget(24 * 1024 * 1024)?;
    for (i, body) in contents.iter().enumerate() {
        let html = crate::webpage::Html::parse_without_text(body, hosts[i])?;
        for link in html.anchor_links().into_iter().take(512) {
            graph.insert(crate::webgraph::Edge {
                from: crate::webgraph::Node::from(html.url()),
                to: crate::webgraph::Node::from(link.destination),
                rel_flags: link.rel,
                label: link.text.chars().take(128).collect(),
                ..crate::webgraph::Edge::empty()
            })?;
        }
    }
    graph.commit()?;
    let computed = crate::webgraph::centrality::harmonic::HarmonicCentrality::calculate(&graph);
    let mut values = computed
        .iter()
        .map(|(node, value)| (*node, value))
        .collect::<Vec<_>>();
    anyhow::ensure!(values.len() <= 4096, "fixture graph exceeds node budget");
    values.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut harmonic: speedy_kv::Db<crate::webgraph::NodeID, f64> =
        speedy_kv::Db::open_or_create(path.join("centrality/harmonic"))?;
    let mut rank: speedy_kv::Db<crate::webgraph::NodeID, u64> =
        speedy_kv::Db::open_or_create(path.join("centrality/harmonic_rank"))?;
    for (i, (node, value)) in values.iter().enumerate() {
        harmonic.insert(*node, *value)?;
        rank.insert(*node, i as u64)?;
    }
    harmonic.commit()?;
    rank.commit()?;
    for (name, count) in [("small", 8), ("medium", 48)] {
        let mut old = WarcWriter::new();
        let mut new = Vec::new();
        for i in 0..count {
            let body = &contents[i % contents.len()];
            let url = format!(
                "{}index-fixture/{i}?signature=keep{i}",
                hosts[i % contents.len()]
            );
            if i % 2 == 0 {
                old.write(&legacy(&url, body.clone()))?;
            } else {
                crate::warc::capture::write(
                    &mut new,
                    &captured(&url, body.as_bytes(), i as i64, false),
                    PayloadType::Html,
                    3,
                    false,
                )?;
            }
        }
        let mut bytes = old.finish()?;
        bytes.extend(new);
        fs::write(path.join(format!("{name}.warc.gz")), bytes)?;
    }
    let text="The original searchable document contains useful words about indexing, recovery and reliable web search. ".repeat(30);
    let url = "https://index.fixture/versions?signature=a%2Fb&order=2&order=1";
    let one =
        format!("<html><title>Version one</title><body><p>obsoletealpha {text}</p></body></html>");
    let two =
        format!("<html><title>Version two</title><body><p>currentbeta {text}</p></body></html>");
    let first = captured(url, one.as_bytes(), 100, false);
    let second = captured(url, two.as_bytes(), 110, false);
    let mut a = Vec::new();
    crate::warc::capture::write(&mut a, &first, PayloadType::Html, 3, false)?;
    crate::warc::capture::write(
        &mut a,
        &captured(
            "https://index.fixture/blocked",
            format!("<html><title>Blocked</title><body><p>noindexmarker {text}</p></body></html>")
                .as_bytes(),
            101,
            true,
        ),
        PayloadType::Html,
        3,
        false,
    )?;
    crate::warc::capture::write(
        &mut a,
        &captured(
            "https://index.fixture/revoked",
            format!("<html><title>Revoked</title><body><p>revokedmarker {text}</p></body></html>")
                .as_bytes(),
            102,
            false,
        ),
        PayloadType::Html,
        3,
        false,
    )?;
    fs::write(path.join("contract-a.warc.gz"), &a)?;
    let mut b = Vec::new();
    crate::warc::capture::write(&mut b, &first, PayloadType::Html, 3, false)?;
    let second_offset = b.len() as u64;
    crate::warc::capture::write(&mut b, &second, PayloadType::Html, 3, false)?;
    let second_end = b.len() as u64;
    crate::warc::capture::write(
        &mut b,
        &captured("https://index.fixture/revoked", b"", 120, true),
        PayloadType::Html,
        3,
        false,
    )?;
    let mut revalidation = captured(url, b"", 130, false);
    revalidation.metadata.response.status = 304;
    revalidation.metadata.revalidation = Some(crate::crawler::recrawl::PayloadReference {
        version: 1,
        fetch_id: second.metadata.fetch_id.clone(),
        segment: "contract-b.warc.gz".into(),
        record_offset: second_offset,
        record_end: second_end,
        payload_sha256: second.metadata.payload_sha256.clone(),
        payload_bytes: second.payload.len(),
        url: url.into(),
    });
    crate::warc::capture::write(&mut b, &revalidation, PayloadType::Html, 3, false)?;
    let mut latin=captured("https://index.fixture/charset",b"<html><title>Charset</title><body><p>Espa\xf1ol idioma documento verificable</p></body></html>",140,false);
    latin.metadata.charset = "windows-1252".into();
    latin
        .metadata
        .response
        .headers
        .push(crate::capture::Header {
            name: "content-type".into(),
            value_hex: crate::capture::hex(b"text/html; charset=windows-1252"),
        });
    latin.metadata.decoded_bytes = encoding_rs::WINDOWS_1252.decode(&latin.payload).0.len();
    crate::warc::capture::write(&mut b, &latin, PayloadType::Html, 3, false)?;
    fs::write(path.join("contract-b.warc.gz"), b)?;
    fs::write(path.join("corrupt.warc.gz"), &a[..a.len() / 2])?;
    let huge = format!(
        "<html><title>Adversarial</title><body><p>{text}</p>{}</body></html>",
        "<a href='/facet?x=1'>bounded anchors</a>".repeat(100000)
    );
    let mut big = WarcWriter::new();
    for i in 0..4 {
        big.write(&legacy(
            &format!("https://index.fixture/large/{i}"),
            huge.clone(),
        ))?;
    }
    fs::write(path.join("large.warc.gz"), big.finish()?)?;
    let mut files = Vec::new();
    for file in fs::read_dir(path)? {
        let file = file?;
        if file.path().extension().is_some_and(|x| x == "gz") {
            let bytes = fs::read(file.path())?;
            files.push(json!({"file":file.file_name().to_string_lossy(),"sha256":crate::capture::sha256(&bytes),"bytes":bytes.len()}));
        }
    }
    let manifest = json!({"version":1,"synthetic_network_requests":0,"signal_provenance":"harmonic centrality calculated from links in retained real HTML; bounded 512 links/document","signal_nodes":values.len(),"ml_models":"not available; disabled","real_html_sources":sources,"cases":["legacy/new WARC","real HTML","charset","large anchors","header noindex","304","duplicate across files","changed version","signed URL","corruption"],"files":files});
    fs::write(
        path.join("corpus.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}
