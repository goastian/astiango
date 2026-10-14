use crate::{
    capture::{self, Capture, Header, HttpResponse, Metadata, Robots},
    warc::PayloadType,
    Result,
};
use serde_json::json;
use std::{fs, path::Path, sync::Arc};
use url::Url;
fn capture(
    url: &str,
    body: &str,
    second: i64,
    header: Option<&str>,
    status: u16,
) -> Result<Capture> {
    let mut headers = vec![Header {
        name: "content-type".into(),
        value_hex: capture::hex(b"text/html; charset=UTF-8"),
    }];
    if let Some(value) = header {
        headers.push(Header {
            name: "x-robots-tag".into(),
            value_hex: capture::hex(value.as_bytes()),
        });
    }
    let response = HttpResponse {
        url: url.into(),
        status,
        version: "HTTP/1.1".into(),
        headers,
        received_at: "2026-10-14T00:00:00Z".parse::<chrono::DateTime<chrono::Utc>>()?
            + chrono::Duration::seconds(second),
        peer_ip: Some("127.0.0.1".parse()?),
        address_validated: true,
        network_policy: "isolated-fixture".into(),
    };
    let robots = Robots::from_headers(&response, capture::DEFAULT_AGENT);
    Ok(Capture {
        metadata: Metadata {
            revalidation: None,
            schema_version: 1,
            identity_version: 1,
            fetch_id: uuid::Uuid::from_u128(100000 + second as u128).to_string(),
            requested_url: capture::url_key(&Url::parse(url)?)?,
            response,
            redirects: vec![],
            payload_sha256: capture::sha256(body.as_bytes()),
            payload_bytes: body.len(),
            decoded_bytes: body.len(),
            charset: "UTF-8".into(),
            decoded_limit_bytes: 4 * 1024 * 1024,
            user_agent: capture::DEFAULT_AGENT.into(),
            robots,
            transformations: vec!["controlled-graph-fixture".into()],
            fetch_time_ms: 1,
            discovery_urls: vec![],
            internal_urls: vec![],
        },
        payload: Arc::new(body.as_bytes().to_vec()),
    })
}
fn add(bytes: &mut Vec<u8>, capture: &Capture) -> Result<()> {
    crate::warc::capture::write(bytes, capture, PayloadType::Html, 3, false)?;
    Ok(())
}
pub fn create(root: &Path) -> Result<serde_json::Value> {
    fs::create_dir_all(root)?;
    let real = root.join("real");
    crate::entrypoint::indexer::fixture::create(&real)?;
    fs::copy(real.join("small.warc.gz"), root.join("small.warc.gz"))?;
    fs::copy(
        real.join("medium.warc.gz"),
        root.join("medium-unique.warc.gz"),
    )?;
    let small = fs::read(root.join("small.warc.gz"))?;
    let mut medium = Vec::new();
    for _ in 0..6 {
        medium.extend(&small);
    }
    fs::write(root.join("medium.warc.gz"), medium)?;
    let body="<html><head><base href='https://base.fixture:8443/a/'><link rel='canonical' href='https://different.fixture/'></head><body><a href='../x?sig=%2F&a=1&a=2'>signed</a><a href='https://münich.example/a/'>IDN</a><a href='https://[2001:db8::1]:8080/x/'>IPv6</a><a href='https://target.fixture/same' rel='nofollow'>one</a><a href='https://target.fixture/same' rel='sponsored'>two</a><a href='https://target.fixture/same'>three</a><a href='mailto:hello@example.org'>mail</a></body></html>";
    let mut contracts = Vec::new();
    for (i,url,header,text) in [
        (1,"https://contract.fixture/base?signature=%2F&fbclid=keep",None,body),
        (2,"https://contract.fixture/noindex",Some("AstianGOBot: noindex"),body),
        (3,"https://contract.fixture/header-nofollow",Some("AstianGOBot: nofollow"),body),
        (4,"https://contract.fixture/meta-nofollow",None,"<meta name='robots' content='nofollow'><a href='https://target.fixture/blocked'>blocked</a>"),
        (5,"https://contract.fixture/meta-noindex",None,"<meta name='robots' content='noindex'><a href='https://target.fixture/allowed'>allowed</a>"),
        (6,"http://tenant.blogspot.com:8080/signed/?sig=%2F&x=2&x=1",None,body),
    ] {add(&mut contracts,&capture(url,text,i,header,200)?)?;}
    let mut redirect = capture("https://redirect.fixture/start", body, 7, None, 200)?;
    redirect.metadata.response.url = "https://redirect.fixture/final/".into();
    let mut hop = redirect.metadata.response.clone();
    hop.url = redirect.metadata.requested_url.clone();
    hop.status = 302;
    hop.headers = vec![Header {
        name: "location".into(),
        value_hex: capture::hex(b"/final/"),
    }];
    redirect.metadata.redirects.push(hop);
    add(&mut contracts, &redirect)?;
    fs::write(root.join("contract.warc.gz"), &contracts)?;
    fs::write(root.join("contract-copy.warc.gz"), &contracts)?;
    let url = "https://versions.fixture/doc";
    let old = capture(
        url,
        "<a href='https://old.fixture/a'>old</a>",
        50,
        None,
        200,
    )?;
    let mut original = Vec::new();
    add(&mut original, &old)?;
    fs::write(root.join("original.warc.gz"), &original)?;
    let mut revalidation = capture(url, "", 60, None, 304)?;
    revalidation.metadata.revalidation = Some(crate::crawler::recrawl::PayloadReference {
        version: 1,
        fetch_id: old.metadata.fetch_id.clone(),
        segment: "original.warc.gz".into(),
        record_offset: 0,
        record_end: original.len() as u64,
        payload_sha256: old.metadata.payload_sha256.clone(),
        payload_bytes: old.metadata.payload_bytes,
        url: url.into(),
    });
    let mut bytes = Vec::new();
    add(&mut bytes, &revalidation)?;
    fs::write(root.join("reference.warc.gz"), bytes)?;
    for (name, time, text, status, header) in [
        (
            "changed",
            70,
            "<a href='https://new.fixture/b'>new</a>",
            200,
            None,
        ),
        ("retired", 80, "", 410, None),
        (
            "policy",
            90,
            "<a href='https://new.fixture/b'>new</a>",
            200,
            Some("nofollow"),
        ),
    ] {
        let mut bytes = Vec::new();
        add(&mut bytes, &capture(url, text, time, header, status)?)?;
        fs::write(root.join(format!("{name}.warc.gz")), bytes)?;
    }
    let mut high = String::from("<html><body>");
    for i in 0..20_000 {
        high.push_str(&format!("<a href='/p/{i}?sig=%2F'>x</a>"));
    }
    high.push_str("</body></html>");
    let mut bytes = Vec::new();
    add(
        &mut bytes,
        &capture("https://high.fixture/", &high, 100, None, 200)?,
    )?;
    fs::write(root.join("adversarial.warc.gz"), bytes)?;
    let mut huge = String::from("<html><body>");
    for i in 0..100_001 {
        huge.push_str(&format!("<a href='/q/{i}'>x</a>"));
    }
    huge.push_str("</body></html>");
    let mut bytes = Vec::new();
    add(
        &mut bytes,
        &capture("https://quota.fixture/", &huge, 101, None, 200)?,
    )?;
    fs::write(root.join("quota.warc.gz"), bytes)?;
    let mut initial = Vec::new();
    for i in 0..64 {
        add(
            &mut initial,
            &capture(
                &format!("https://incremental.fixture/{i}"),
                &format!("<a href='https://target.fixture/{i}'>value{i}</a>"),
                200 + i,
                None,
                200,
            )?,
        )?;
    }
    fs::write(root.join("incremental-initial.warc.gz"), initial)?;
    let mut scale = Vec::new();
    for i in 0..640 {
        add(
            &mut scale,
            &capture(
                &format!("https://scale.fixture/{i}"),
                &format!("<a href='https://target.fixture/{i}'>value{i}</a>"),
                1000 + i,
                None,
                200,
            )?,
        )?;
    }
    fs::write(root.join("scale-10x.warc.gz"), scale)?;
    let mut changes = Vec::new();
    for i in 0..4 {
        add(
            &mut changes,
            &capture(
                &format!("https://incremental.fixture/{i}"),
                &format!("<a href='https://changed.fixture/{i}'>change{i}</a>"),
                300 + i,
                None,
                200,
            )?,
        )?;
    }
    fs::write(root.join("incremental-changed.warc.gz"), changes)?;
    fs::write(
        root.join("corrupt.warc.gz"),
        &contracts[..contracts.len() / 2],
    )?;
    let mut files = Vec::new();
    for file in fs::read_dir(root)? {
        let file = file?;
        if file.path().extension().and_then(|v| v.to_str()) == Some("gz") {
            files.push(json!({"file":file.file_name().to_string_lossy(),"bytes":file.metadata()?.len(),"sha256":super::consumer::checksum(fs::File::open(file.path())?)?}));
        }
    }
    files.sort_by(|a, b| a["file"].as_str().cmp(&b["file"].as_str()));
    let value = json!({"version":1,"files":files,"real_html":serde_json::from_slice::<serde_json::Value>(&fs::read(real.join("corpus.json"))?)?["real_html_sources"],"favorable_condition":"medium repeats eight real documents six times; medium-unique and adversarial are separate","contracts":["legacy/new","base","redirect","header/meta noindex/nofollow","304 original reference","signed queries","IDN/IPv6/ports/private suffix","cross-input duplicate","corruption","100001 anchors","changes/withdrawal/policy"],"external_requests":0});
    fs::write(root.join("corpus.json"), serde_json::to_vec_pretty(&value)?)?;
    Ok(value)
}
