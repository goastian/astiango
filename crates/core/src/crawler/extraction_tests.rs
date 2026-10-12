use super::*;

#[test]
fn capture_identity_retains_queries_while_the_legacy_entry_keeps_its_contract() {
    let url = Url::parse("https://example.org/").unwrap();
    let domain = Domain::from(&url);
    let visited = VisitedUrls::new();
    let context = || ExtractionContext {
        domain: &domain,
        crawled_urls: &visited,
        requested_url: &url,
    };
    let body = "<html><body><a href='/page?b=2&amp;utm_source=one&amp;a=%2f#part'>one</a><a href='/page?b=2&amp;utm_source=two&amp;a=%2f'>two</a></body></html>";
    let captured =
        CrawlExtraction::parse_capture(body, &url, context(), crate::capture::DEFAULT_AGENT)
            .unwrap();
    assert_eq!(captured.internal_urls.len(), 2);
    assert_eq!(
        captured.internal_urls[0].as_str(),
        "https://example.org/page?b=2&utm_source=one&a=%2f"
    );
    let legacy = compare(body, &url, &visited);
    assert_eq!(legacy.internal_urls.len(), 1);
}

fn compare(body: &str, url: &Url, visited: &VisitedUrls) -> CrawlExtraction {
    let domain = Domain::from(url);
    let context = || ExtractionContext {
        domain: &domain,
        crawled_urls: visited,
        requested_url: url,
    };
    let old = legacy_double_parse(body, url, context()).unwrap();
    let new = CrawlExtraction::parse(body, url, context()).unwrap();
    assert_eq!(new.url, old.url);
    assert_eq!(new.base_url, old.base_url);
    assert_eq!(new.canonical_url, old.canonical_url);
    assert_eq!(new.no_index, old.no_index);
    assert_eq!(new.no_follow, old.no_follow);
    assert_eq!(new.discovery_urls, old.discovery_urls);
    assert_eq!(new.internal_urls, old.internal_urls);
    new
}

#[test]
fn conserved_c01_corpus_preserves_order_base_robots_and_both_projections() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest: serde_json::Value = serde_json::from_slice(
        &std::fs::read(root.join("tests/crawler-bench/manifest.json")).unwrap(),
    )
    .unwrap();
    for fixture in manifest["fixtures"].as_array().unwrap() {
        let bytes = std::fs::read(root.join(fixture["path"].as_str().unwrap())).unwrap();
        let body = String::from_utf8_lossy(&bytes);
        let url = Url::parse(
            fixture["source_url"]
                .as_str()
                .unwrap_or("https://example.org/article"),
        )
        .unwrap();
        compare(&body, &url, &VisitedUrls::default());
    }
}

#[test]
fn semantic_edges_preserve_resolution_normalization_filters_and_robots() {
    let url = Url::parse("https://example.org/current").unwrap();
    let visited = VisitedUrls::from_iter([Url::parse("https://example.org/base/seen").unwrap()]);
    let body = r#"<html><head><base href="/base/"><link rel="canonical" href="canonical">
        <meta name="robots" content="noindex"></head><body>
        <a href="../current">self</a><a href="one#fragment">one <b>bold</b></a>
        <a href="one">duplicate</a><a href="seen">seen</a><a href="file.pdf">pdf</a>
        <a href="//other.org/">external</a><a href="mailto:mail@example.org">mail</a>
        <a href="http://[invalid">invalid</a><a href="two" rel="nofollow">two</a>
        <a href="broken"><a href="nested">parser repair</a></body></html>"#;
    let result = compare(body, &url, &visited);
    assert_eq!(result.base_url.as_str(), "https://example.org/base/");
    assert!(result.no_index);
    assert!(!result.no_follow);
    assert!(result
        .internal_urls
        .iter()
        .any(|url| url.path() == "/base/two"));
    assert!(!result
        .internal_urls
        .iter()
        .any(|url| url.path().ends_with(".pdf")));
    assert!(!result
        .internal_urls
        .iter()
        .any(|url| url.path() == "/base/seen"));
}

#[test]
fn nofollow_suppresses_both_projections_but_retains_noindex_and_base() {
    let url = Url::parse("https://example.org/").unwrap();
    let result = compare(
        r#"<base href="https://base.example.org/"><meta name="robots" content="nofollow,noindex"><a href="one">one</a>"#,
        &url,
        &VisitedUrls::default(),
    );
    assert!(result.no_follow && result.no_index);
    assert!(result.internal_urls.is_empty() && result.discovery_urls.is_empty());
    assert_eq!(result.anchors_visited, 0);
}

#[test]
fn internal_quota_remains_independent_of_external_prefix_and_duplicates() {
    let url = Url::parse("https://example.org/").unwrap();
    let mut body = "<a href='https://other.org/'>external</a>".repeat(600);
    body.push_str("<a href='/first'>first</a><a href='/first#fragment'>duplicate</a><a href='/second'>second</a>");
    let result = compare(&body, &url, &VisitedUrls::default());
    assert_eq!(result.discovery_urls.len(), MAX_OUTGOING_URLS_PER_PAGE);
    assert_eq!(
        result
            .internal_urls
            .iter()
            .map(Url::path)
            .collect::<Vec<_>>(),
        ["/first", "/second"]
    );
    assert!(result.discovery_limit_reached);
    assert!(!result.traversal_stopped);
}

#[test]
fn three_hundred_thousand_anchors_stop_at_the_retained_internal_quota() {
    let url = Url::parse("https://example.org/").unwrap();
    let mut body = String::with_capacity(12_000_000);
    for n in 0..300_000 {
        body.push_str(&format!("<a href='/page-{n}'>anchor {n}</a>"));
    }
    let domain = Domain::from(&url);
    let result = CrawlExtraction::parse(
        &body,
        &url,
        ExtractionContext {
            domain: &domain,
            crawled_urls: &VisitedUrls::default(),
            requested_url: &url,
        },
    )
    .unwrap();
    assert_eq!(result.discovery_urls.len(), MAX_OUTGOING_URLS_PER_PAGE);
    assert_eq!(result.internal_urls.len(), MAX_OUTGOING_URLS_PER_PAGE);
    assert_eq!(result.anchors_visited, MAX_OUTGOING_URLS_PER_PAGE as u64);
    assert!(result.traversal_stopped);
    assert_eq!(result.internal_urls.last().unwrap().path(), "/page-511");
}

#[test]
fn cloned_extractor_keeps_one_shared_count_per_document() {
    let extractor = Extractor::default();
    let url = Url::parse("https://example.org/").unwrap();
    let domain = Domain::from(&url);
    for instance in [&extractor, &extractor.clone()] {
        instance
            .parse(
                "<a href='/next'>next</a>",
                &url,
                ExtractionContext {
                    domain: &domain,
                    crawled_urls: &VisitedUrls::default(),
                    requested_url: &url,
                },
            )
            .unwrap();
    }
    let stats = extractor.stats();
    assert_eq!(stats.documents, 2);
    assert_eq!(stats.dom_parses, 2);
    assert_eq!(stats.internal_links, 2);
    assert_eq!(stats.failures, 0);
    #[cfg(target_os = "linux")]
    assert_eq!(stats.cpu_samples, 2);
}
