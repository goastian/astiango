use super::*;
use std::io::Write;

fn header(name: &str, value: &str) -> Header {
    Header {
        name: name.into(),
        value_hex: hex(value.as_bytes()),
    }
}

#[test]
fn identity_keeps_query_order_encoding_empty_query_and_origins() {
    for value in [
        "https://EXAMPLE.org:443/a?b=2&a=1&a=3#fragment",
        "https://example.org/a?utm_source=original",
        "https://example.org/a?",
        "https://example.org/%2Fa?q=%20",
        "http://example.org:8080/a",
    ] {
        let url = Url::parse(value).unwrap();
        let mut expected = url.clone();
        expected.set_fragment(None);
        assert_eq!(url_key(&url).unwrap(), expected.as_str());
    }
    assert_ne!(
        url_key(&Url::parse("http://example.org/a").unwrap()).unwrap(),
        url_key(&Url::parse("https://example.org/a").unwrap()).unwrap()
    );
    assert!(url_key(&Url::parse("https://user:password@example.org/").unwrap()).is_err());
}

#[test]
fn robots_preserves_scopes_multiple_headers_and_restrictive_rules() {
    let url = Url::parse("https://example.org/").unwrap();
    let capture = fixture(
        &url,
        uuid::Uuid::new_v4().to_string(),
        b"html".to_vec(),
        vec![
            header("X-Robots-Tag", "otherbot: noindex, nofollow"),
            header("x-robots-tag", "ASTIANGOBOT: index, nofollow"),
            header("x-robots-tag", "noindex, index, max-snippet:50"),
        ],
    );
    let robots = Robots::from_headers(&capture.metadata.response, DEFAULT_AGENT);
    assert!(robots.no_index && robots.no_follow);
    assert_eq!(capture.metadata.response.values("x-robots-tag").count(), 3);
    let mut scoped = fixture(
        &url,
        uuid::Uuid::new_v4().to_string(),
        b"html".to_vec(),
        vec![header("x-robots-tag", "otherbot: none")],
    )
    .metadata
    .robots;
    assert!(!scoped.no_index && !scoped.no_follow);
    scoped.add(
        Directive {
            agent: Some("AstianGOBot".into()),
            source: "html".into(),
            value: "NONE, index".into(),
        },
        DEFAULT_AGENT,
    );
    assert!(scoped.no_index && scoped.no_follow);
}

#[test]
fn new_warc_round_trips_raw_gzip_and_legacy_reader_still_works() {
    let body = "<html><title>Original</title><body>gzip original</body></html>";
    let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gzip.write_all(body.as_bytes()).unwrap();
    let raw = gzip.finish().unwrap();
    let url = Url::parse("https://example.org/final").unwrap();
    let mut capture = fixture(
        &url,
        uuid::Uuid::new_v4().to_string(),
        raw.clone(),
        vec![
            header("content-type", "text/html; charset=UTF-8"),
            header("content-encoding", "gzip"),
            header("x-robots-tag", "noindex"),
        ],
    );
    capture.metadata.requested_url = "https://example.org/requested?utm_source=original".into();
    let mut output = Vec::new();
    let offset = crate::warc::capture::write(
        &mut output,
        &capture,
        crate::warc::PayloadType::Html,
        3,
        true,
    )
    .unwrap();
    assert!(offset > 0);
    let records: Vec<_> = crate::warc::WarcFile::new(output.clone())
        .records()
        .collect::<crate::Result<_>>()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].response.body, body);
    assert_eq!(records[0].request.url, url.as_str());
    assert!(!records[0].indexing_allowed());
    assert!(records[0].links_allowed());
    assert_eq!(
        records[0].metadata.capture.as_ref().unwrap().payload_sha256,
        sha256(&raw)
    );
    let mut physical = Vec::new();
    flate2::read::MultiGzDecoder::new(output.as_slice())
        .read_to_end(&mut physical)
        .unwrap();
    assert!(physical.windows(raw.len()).any(|window| window == raw));
    assert!(String::from_utf8_lossy(&physical).contains("WARC/1.1"));
    assert!(physical
        .windows(b"HTTP/1.1 200 \r\n".len())
        .any(|window| window == b"HTTP/1.1 200 \r\n"));
}

#[test]
fn unknown_version_and_corrupt_payload_fail_with_contract_errors() {
    let url = Url::parse("https://example.org/").unwrap();
    let mut capture = fixture(
        &url,
        uuid::Uuid::new_v4().to_string(),
        b"body".to_vec(),
        Vec::new(),
    );
    capture.metadata.schema_version = 99;
    assert!(capture
        .metadata
        .validate()
        .unwrap_err()
        .to_string()
        .contains("unsupported capture schema version 99"));
    assert!(crate::warc::capture::write(
        &mut Vec::new(),
        &capture,
        crate::warc::PayloadType::Html,
        3,
        false
    )
    .is_err());
    capture.metadata.schema_version = 1;
    capture.payload = Arc::new(b"evil".to_vec());
    assert!(crate::warc::capture::write(
        &mut Vec::new(),
        &capture,
        crate::warc::PayloadType::Html,
        3,
        false
    )
    .unwrap_err()
    .to_string()
    .contains("payload mismatch"));
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"WARC/1.1\r\nWARC-Type: response\r\nAstian-Capture-Version: 99\r\nContent-Length: 0\r\n\r\n\r\n\r\n").unwrap();
    assert!(crate::warc::WarcFile::new(encoder.finish().unwrap())
        .records()
        .next()
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("unsupported capture schema version 99"));
}

#[test]
fn compression_bombs_and_multibyte_charset_are_bounded() {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&vec![b'x'; 100_000]).unwrap();
    assert!(decode(&encoder.finish().unwrap(), "gzip", "UTF-8", 4096)
        .unwrap_err()
        .to_string()
        .contains("exceeds limit"));
    assert_eq!(decode(&[0xe9], "identity", "windows-1252", 3).unwrap(), "é");
    assert!(decode(&[0xe9; 4], "identity", "windows-1252", 4).is_err());
}

#[test]
fn robots_decision_covers_rules_after_the_metadata_retention_bound() {
    let mut body = "<html><head>".to_owned();
    for _ in 0..1000 {
        body.push_str("<meta name=AstianGOBot content=index>");
    }
    body.push_str("<meta name=AstianGOBot content=none></head><body>Original</body></html>");
    let html = crate::webpage::Html::parse_without_text(&body, "https://example.org/").unwrap();
    let robots = html.robots_for_agent(DEFAULT_AGENT);
    assert!(robots.no_index && robots.no_follow && robots.directives_truncated);
    assert_eq!(robots.directives.len(), 64);
    assert_eq!(robots.directives_seen, 1001);
    assert!(!html.robots_for_agent("otherbot").no_index);
}

#[test]
fn a_contract_error_ends_iteration_and_does_not_parse_payload_as_headers() {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"WARC/1.1\r\nWARC-Type: response\r\nAstian-Capture-Version: 99\r\nContent-Length: 5\r\n\r\nbytes\r\n\r\n").unwrap();
    let bytes = encoder.finish().unwrap();
    let file = crate::warc::WarcFile::new(bytes);
    let mut reader = file.records();
    assert!(reader
        .next()
        .unwrap()
        .unwrap_err()
        .to_string()
        .contains("unsupported capture schema version 99"));
    assert!(reader.next().is_none());
}
