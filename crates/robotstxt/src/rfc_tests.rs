use super::*;

#[test]
fn groups_merge_without_prefix_agent_leakage() {
    let rules = Robots::parse_rfc("ExampleBot", "Disallow: /ungrouped\nUser-agent: *\nDisallow: /\nUser-agent: examplebot\nDisallow: /a\nSitemap: https://example.com/map\nUser-agent: OtherBot\nDisallow: /b\nUser-agent: ExampleBot\nDisallow: /c\nAllow: /a/public\nUser-agent: ExampleBotOther\nDisallow: /d\n").unwrap();
    for path in ["/b", "/d", "/ungrouped", "/a/public"] {
        assert!(rules.is_path_allowed(path), "{path}");
    }
    for path in ["/a", "/c"] {
        assert!(!rules.is_path_allowed(path), "{path}");
    }
}

#[test]
fn escapes_wildcards_precedence_and_exact_paths() {
    let rules = Robots::parse_rfc("bot", "User-agent: *\nDisallow: /\nAllow: /a%62\nAllow: /literal%2A$\nAllow: /a/b$\nAllow: /utf/ツ$\nAllow: /x/*/end$\nAllow: /equal\nDisallow: /equal\nAllow: /directory/index.html\n").unwrap();
    for path in [
        "/ab",
        "/a%62",
        "/literal*",
        "/literal%2a",
        "/utf/%E3%83%84",
        "/x/one/two/end",
        "/equal",
    ] {
        assert!(rules.is_path_allowed(path), "{path}");
    }
    for path in [
        "/a%2Fb",
        "/a//b",
        "/literal-more",
        "/directory/",
        "/x/one/end/extra",
    ] {
        assert!(!rules.is_path_allowed(path), "{path}");
    }
    assert!(rules.is_allowed(&Url::parse("https://example.org/a/b").unwrap()));
    assert!(!rules.is_allowed(&Url::parse("https://example.org/a//b").unwrap()));
}

#[test]
fn parseable_lines_after_errors_and_large_utf8_file_survive() {
    let mut body = "User-agent: *\n".to_owned();
    body.push_str(&"#éééééééé\n".repeat(28_000));
    body.push_str("invalid line\nDisallow: /late\nCrawl-delay: 7.5\n");
    assert!(body.len() > 500_000 && body.len() < 512_000);
    let rules = Robots::parse_rfc("bot", &body).unwrap();
    assert!(!rules.is_path_allowed("/late"));
    assert_eq!(rules.crawl_delay(), Some(Duration::from_millis(7500)));
    assert!(Robots::parse_rfc_with_params(
        "bot",
        &body,
        Params {
            char_limit: 500_000
        }
    )
    .is_err());
}

#[test]
fn normative_corpus_agrees_with_legacy_on_supported_common_cases() {
    for body in [
        "User-agent: *\nDisallow: /private\nAllow: /private/public\n",
        "User-agent: bot\nDisallow: /a\nUser-agent: bot\nDisallow: /b\n",
        "User-agent: *\nDisallow: /*.pdf$\n",
    ] {
        let old = Robots::parse("bot", body).unwrap();
        let new = Robots::parse_rfc("bot", body).unwrap();
        for path in [
            "/",
            "/a",
            "/b",
            "/private",
            "/private/public",
            "/doc.pdf",
            "/doc.pdf?x=1",
        ] {
            assert_eq!(
                new.is_path_allowed(path),
                old.is_path_allowed(path),
                "{body}: {path}"
            );
        }
    }
}
