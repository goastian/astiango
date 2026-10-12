use super::*;
use std::{convert::Infallible, task::Poll};

fn config(wire: usize, decoded: usize, concurrent: usize) -> BodyConfig {
    let limits = BodyLimits {
        wire_limit_bytes: wire,
        decoded_limit_bytes: decoded,
    };
    BodyConfig {
        html: limits.clone(),
        feed: limits.clone(),
        sitemap: limits.clone(),
        robots: limits,
        max_inflight_bodies: concurrent,
        max_reserved_bytes: (wire + 3 * decoded) * concurrent,
        idle_timeout_ms: 200,
        total_timeout_ms: 1000,
        budget_wait_timeout_ms: 100,
        ..Default::default()
    }
}

fn response(status: u16, headers: &[(&str, &str)], chunks: Vec<Vec<u8>>) -> reqwest::Response {
    let mut builder = http_02::Response::builder().status(status);
    for (key, value) in headers {
        builder = builder.header(*key, *value);
    }
    let stream = futures::stream::iter(chunks.into_iter().map(Ok::<_, Infallible>));
    builder
        .body(reqwest::Body::wrap_stream(stream))
        .unwrap()
        .into()
}

fn pending_response(
    status: u16,
    headers: &[(&str, &str)],
    polls: Arc<AtomicU64>,
) -> reqwest::Response {
    let mut builder = http_02::Response::builder().status(status);
    for (key, value) in headers {
        builder = builder.header(*key, *value);
    }
    let stream = futures::stream::poll_fn(move |_| {
        polls.fetch_add(1, Ordering::Relaxed);
        Poll::<Option<Result<Vec<u8>, Infallible>>>::Pending
    });
    builder
        .body(reqwest::Body::wrap_stream(stream))
        .unwrap()
        .into()
}

fn html(text: &str) -> reqwest::Response {
    response(
        200,
        &[("content-type", "text/html")],
        vec![text.as_bytes().to_vec()],
    )
}

#[tokio::test]
async fn admission_rejects_status_mime_encoding_and_declared_size_without_polling() {
    let reader = BodyReader::new(config(16, 32, 1)).unwrap();
    for (status, headers, reason) in [
        (404, vec![("content-type", "text/html")], "status_rejected"),
        (302, vec![("location", "/next")], "status_rejected"),
        (
            200,
            vec![("content-type", "image/jpeg")],
            "unsupported_mime",
        ),
        (200, vec![], "unsupported_mime"),
        (
            200,
            vec![("content-type", "text/html"), ("content-encoding", "br")],
            "unsupported_encoding",
        ),
        (
            200,
            vec![
                ("content-type", "text/html"),
                ("content-encoding", "identity, br"),
            ],
            "unsupported_encoding",
        ),
        (
            200,
            vec![("content-type", "text/html"), ("content-length", "17")],
            "wire_limit_exceeded",
        ),
        (
            200,
            vec![("content-type", "text/html"), ("content-length", "-1")],
            "invalid_content_length",
        ),
        (
            200,
            vec![("content-type", "text/html"), ("content-length", "1, 2")],
            "invalid_content_length",
        ),
    ] {
        let polls = Arc::new(AtomicU64::new(0));
        let result = reader
            .read(
                pending_response(status, &headers, polls.clone()),
                BodyKind::Page,
            )
            .await;
        assert_eq!(result.err().unwrap().reason(), reason);
        assert_eq!(polls.load(Ordering::Relaxed), 0);
    }
    let stats = reader.stats();
    assert_eq!(stats.bodies_admitted, 0);
    assert_eq!(stats.wire_bytes_received, 0);
    assert_eq!(stats.active_bodies, 0);
    assert_eq!(stats.rejected_status, 2);
    assert_eq!(stats.rejected_encoding, 2);
    assert_eq!(stats.rejected_content_length, 3);
}

#[tokio::test]
async fn all_content_encoding_headers_are_checked_and_identity_is_supported() {
    let reader = BodyReader::new(config(16, 32, 1)).unwrap();
    let body = reader
        .read(
            response(
                200,
                &[
                    ("content-type", "text/html"),
                    ("content-encoding", "IDENTITY, identity"),
                ],
                vec![b"ok".to_vec()],
            ),
            BodyKind::Html,
        )
        .await
        .unwrap();
    assert_eq!(body.as_str(), "ok");
    drop(body);
    let polls = Arc::new(AtomicU64::new(0));
    let response = pending_response(
        200,
        &[
            ("content-type", "text/html"),
            ("content-encoding", "identity"),
            ("content-encoding", "deflate"),
        ],
        polls.clone(),
    );
    assert_eq!(
        reader
            .read(response, BodyKind::Html)
            .await
            .err()
            .unwrap()
            .reason(),
        "unsupported_encoding"
    );
    assert_eq!(polls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn incremental_charset_and_bom_decoding_preserves_chunk_boundaries() {
    let reader = BodyReader::new(config(64, 64, 1)).unwrap();
    let cases = [
        (
            "text/html; charset=utf-8",
            vec![vec![0xe2], vec![0x82], vec![0xac]],
            "€",
        ),
        (
            "text/html; charset=windows-1252",
            vec![vec![0x80], vec![0xe9]],
            "€é",
        ),
        (
            "text/html; charset=utf-8",
            vec![vec![0xff], vec![0xfe, b'A'], vec![0]],
            "A",
        ),
        ("text/html; charset=utf-8", vec![vec![0xe2], vec![]], "�"),
        (
            "text/html; charset=unknown",
            vec![b"ascii".to_vec()],
            "ascii",
        ),
    ];
    for (mime, chunks, expected) in cases {
        let body = reader
            .read(
                response(200, &[("content-type", mime)], chunks),
                BodyKind::Html,
            )
            .await
            .unwrap();
        assert_eq!(body.as_str(), expected);
        assert_eq!(body.payload_type(), Some(PayloadType::Html));
        assert_eq!(body.kind(), BodyKind::Html);
    }
    assert_eq!(reader.stats().bodies_completed, 5);
    assert_eq!(reader.stats().reserved_bytes, 0);
}

#[tokio::test]
async fn wire_overflow_drops_the_chunk_before_decoding_or_appending() {
    let reader = BodyReader::new(config(8, 32, 1)).unwrap();
    let result = reader
        .read(
            response(
                200,
                &[("content-type", "text/html")],
                vec![b"1234".to_vec(), b"56789".to_vec()],
            ),
            BodyKind::Html,
        )
        .await;
    assert_eq!(result.err().unwrap().reason(), "wire_limit_exceeded");
    assert_eq!(reader.stats().wire_bytes_received, 9);
    assert_eq!(reader.stats().decoded_bytes_produced, 0);
    assert_eq!(reader.stats().active_bodies, 0);
    assert_eq!(reader.stats().reserved_bytes, 0);
}

#[tokio::test]
async fn decoded_expansion_has_its_own_limit_and_exact_limit_is_accepted() {
    let reader = BodyReader::new(config(8, 6, 1)).unwrap();
    let body = reader
        .read(
            response(
                200,
                &[("content-type", "text/html; charset=windows-1252")],
                vec![vec![0x80], vec![0x80]],
            ),
            BodyKind::Html,
        )
        .await
        .unwrap();
    assert_eq!(body.as_str(), "€€");
    drop(body);
    let reader = BodyReader::new(config(8, 5, 1)).unwrap();
    let result = reader
        .read(
            response(
                200,
                &[("content-type", "text/html; charset=windows-1252")],
                vec![vec![0x80], vec![0x80]],
            ),
            BodyKind::Html,
        )
        .await;
    assert_eq!(result.err().unwrap().reason(), "decoded_limit_exceeded");
    assert_eq!(reader.stats().wire_bytes_received, 2);
    assert_eq!(reader.stats().decoded_bytes_produced, 0);
    assert_eq!(reader.stats().reserved_bytes, 0);
}

#[tokio::test]
async fn declared_lengths_cannot_mask_overflow_or_truncation() {
    let reader = BodyReader::new(config(8, 32, 1)).unwrap();
    for (length, data, expected) in [
        ("2", b"123456789".as_slice(), "wire_limit_exceeded"),
        ("6", b"123".as_slice(), "content_length_mismatch"),
        ("2", b"123".as_slice(), "content_length_mismatch"),
    ] {
        let result = reader
            .read(
                response(
                    200,
                    &[("content-type", "text/html"), ("content-length", length)],
                    vec![data.to_vec()],
                ),
                BodyKind::Html,
            )
            .await;
        assert_eq!(result.err().unwrap().reason(), expected);
    }
    let body = reader
        .read(
            response(
                200,
                &[("content-type", "text/html"), ("content-length", "0")],
                vec![],
            ),
            BodyKind::Html,
        )
        .await
        .unwrap();
    assert!(body.is_empty());
}

#[tokio::test]
async fn page_selects_feed_limits_and_compatibility_mimes_are_explicit() {
    let mut cfg = config(16, 32, 1);
    cfg.html.wire_limit_bytes = 2;
    let reader = BodyReader::new(cfg).unwrap();
    for (mime, expected) in [
        ("application/rss+xml", PayloadType::Rss),
        ("application/atom+xml", PayloadType::Atom),
    ] {
        let body = reader
            .read(
                response(200, &[("content-type", mime)], vec![b"feed".to_vec()]),
                BodyKind::Page,
            )
            .await
            .unwrap();
        assert_eq!(body.kind(), BodyKind::Feed);
        assert_eq!(body.payload_type(), Some(expected));
    }
    for (kind, mime) in [
        (BodyKind::Robots, None),
        (BodyKind::Robots, Some("text/html")),
        (BodyKind::Robots, Some("text/plain")),
        (BodyKind::Feed, None),
        (BodyKind::Feed, Some("text/plain")),
        (BodyKind::Feed, Some("application/xml")),
        (BodyKind::Sitemap, None),
        (BodyKind::Sitemap, Some("text/xml")),
    ] {
        let headers: Vec<_> = mime
            .into_iter()
            .map(|mime| ("content-type", mime))
            .collect();
        assert!(reader
            .read(response(200, &headers, vec![b"x".to_vec()]), kind)
            .await
            .is_ok());
    }
    assert!(matches!(
        reader
            .read(
                response(200, &[("content-type", "application/xml")], vec![]),
                BodyKind::Page
            )
            .await,
        Err(BodyError::UnsupportedMime(_))
    ));
}

#[tokio::test]
async fn reservations_survive_into_parts_until_the_consumer_releases_the_lease() {
    let mut cfg = config(16, 32, 2);
    cfg.max_reserved_bytes = 16 + 3 * 32;
    cfg.budget_wait_timeout_ms = 20;
    let reader = BodyReader::new(cfg).unwrap();
    let first = reader.read(html("one"), BodyKind::Html).await.unwrap();
    let (text, lease) = first.into_parts();
    let polls = Arc::new(AtomicU64::new(0));
    let result = reader
        .read(
            pending_response(200, &[("content-type", "text/html")], polls.clone()),
            BodyKind::Html,
        )
        .await;
    assert_eq!(result.err().unwrap().reason(), "budget_timeout");
    assert_eq!(polls.load(Ordering::Relaxed), 0);
    assert_eq!(reader.stats().active_bodies, 1);
    assert_eq!(reader.stats().reserved_bytes, 112);
    drop(text);
    drop(lease);
    assert!(reader.read(html("two"), BodyKind::Html).await.is_ok());
    assert_eq!(reader.stats().active_bodies, 0);
    assert_eq!(reader.stats().peak_reserved_bytes, 112);
    assert_eq!(reader.stats().budget_waits, 1);
}

#[tokio::test]
async fn count_budget_waiters_resume_and_cancellation_does_not_leak_reservations() {
    let reader = BodyReader::new(config(16, 32, 1)).unwrap();
    let first = reader.read(html("held"), BodyKind::Html).await.unwrap();
    let task = {
        let reader = reader.clone();
        tokio::spawn(async move { reader.read(html("waiting"), BodyKind::Html).await })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        while reader.stats().budget_waits == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(reader.stats().bodies_admitted, 1);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(reader.stats().active_bodies, 1);
    let second = {
        let reader = reader.clone();
        tokio::spawn(async move { reader.read(html("resumed"), BodyKind::Html).await })
    };
    drop(first);
    let body = second.await.unwrap().unwrap();
    assert_eq!(body.as_str(), "resumed");
    drop(body);
    let polls = Arc::new(AtomicU64::new(0));
    let task = {
        let reader = reader.clone();
        let polls = polls.clone();
        tokio::spawn(async move {
            reader
                .read(
                    pending_response(200, &[("content-type", "text/html")], polls),
                    BodyKind::Html,
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        while polls.load(Ordering::Relaxed) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(reader.stats().active_bodies, 1);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(reader.stats().reserved_bytes, 0);
}

#[tokio::test]
async fn idle_timeout_and_original_request_deadline_are_distinct() {
    let mut cfg = config(16, 32, 1);
    cfg.idle_timeout_ms = 20;
    let reader = BodyReader::new(cfg).unwrap();
    let polls = Arc::new(AtomicU64::new(0));
    assert_eq!(
        reader
            .read(
                pending_response(200, &[("content-type", "text/html")], polls.clone()),
                BodyKind::Html
            )
            .await
            .err()
            .unwrap()
            .reason(),
        "idle_timeout"
    );
    let mut response = pending_response(200, &[("content-type", "text/html")], polls.clone());
    response.extensions_mut().insert(RequestDeadline {
        started: Instant::now() - Duration::from_millis(100),
        expires: Instant::now() - Duration::from_millis(1),
    });
    let before = polls.load(Ordering::Relaxed);
    assert_eq!(
        reader
            .read(response, BodyKind::Html)
            .await
            .err()
            .unwrap()
            .reason(),
        "total_timeout"
    );
    assert_eq!(polls.load(Ordering::Relaxed), before);
    assert_eq!(reader.stats().idle_timeouts, 1);
    assert_eq!(reader.stats().total_timeouts, 1);
    assert_eq!(reader.stats().reserved_bytes, 0);
}

#[tokio::test]
async fn trickling_bytes_do_not_restart_the_total_deadline() {
    let mut cfg = config(64, 64, 1);
    cfg.idle_timeout_ms = 50;
    cfg.total_timeout_ms = 65;
    let reader = BodyReader::new(cfg).unwrap();
    let stream = futures::stream::unfold(0, |count| async move {
        tokio::time::sleep(Duration::from_millis(10)).await;
        Some((Ok::<_, Infallible>(b"x".to_vec()), count + 1))
    });
    let response = http_02::Response::builder()
        .header("content-type", "text/html")
        .body(reqwest::Body::wrap_stream(stream))
        .unwrap()
        .into();
    assert_eq!(
        reader
            .read(response, BodyKind::Html)
            .await
            .err()
            .unwrap()
            .reason(),
        "total_timeout"
    );
    assert!(reader.stats().wire_bytes_received > 1);
    assert_eq!(reader.stats().reserved_bytes, 0);
}

#[test]
fn impossible_reservations_and_invalid_timeouts_are_rejected_at_construction() {
    let mut cfg = BodyConfig::default();
    cfg.max_reserved_bytes = 1;
    assert!(BodyReader::new(cfg).is_err());
    let mut cfg = BodyConfig::default();
    cfg.html.decoded_limit_bytes = usize::MAX;
    assert!(BodyReader::new(cfg).is_err());
    let mut cfg = BodyConfig::default();
    cfg.idle_timeout_ms = 0;
    assert!(BodyReader::new(cfg).is_err());
    assert!(BodyReader::new(BodyConfig::default()).is_ok());
    assert!(BodyError::StatusRejected(429).is_retryable());
    assert!(!BodyError::UnsupportedEncoding("gzip".into()).is_retryable());
}

#[tokio::test]
#[allow(deprecated)]
async fn legacy_decoder_preserves_status_and_mime_compatibility_with_bounded_decoding() {
    for (status, mime) in [
        (200, None),
        (200, Some("text/plain")),
        (200, Some("application/xml")),
        (404, Some("text/plain")),
        (200, Some("invalid mime")),
    ] {
        let headers: Vec<_> = mime
            .into_iter()
            .map(|mime| ("content-type", mime))
            .collect();
        let text = super::super::encoded_body(response(status, &headers, vec![b"legacy".to_vec()]))
            .await
            .unwrap();
        assert_eq!(text, "legacy");
    }
    let polls = Arc::new(AtomicU64::new(0));
    let error = super::super::encoded_body(pending_response(
        404,
        &[("content-encoding", "br")],
        polls.clone(),
    ))
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        super::super::Error::Body(BodyError::UnsupportedEncoding(_))
    ));
    let error = super::super::encoded_body(pending_response(
        200,
        &[("content-length", "33554433")],
        polls.clone(),
    ))
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        super::super::Error::Body(BodyError::WireLimitExceeded { .. })
    ));
    assert_eq!(polls.load(Ordering::Relaxed), 0);
    let reader = BodyReader::new(config(8, 5, 1)).unwrap();
    assert!(matches!(
        reader
            .read_legacy(response(
                404,
                &[("content-type", "application/xml; charset=windows-1252")],
                vec![vec![0x80, 0x80]]
            ))
            .await,
        Err(BodyError::DecodedLimitExceeded { .. })
    ));
    assert_eq!(reader.stats().reserved_bytes, 0);
    let mut response = pending_response(200, &[], polls.clone());
    response.extensions_mut().insert(RequestDeadline {
        started: Instant::now() - Duration::from_millis(100),
        expires: Instant::now() - Duration::from_millis(1),
    });
    assert!(matches!(
        reader.read_legacy(response).await,
        Err(BodyError::TotalTimeout)
    ));
    assert_eq!(polls.load(Ordering::Relaxed), 0);
}
