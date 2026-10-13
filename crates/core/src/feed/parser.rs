// Stract is an open source web search engine.
// Copyright (C) 2023 Stract ApS
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as
// published by the Free Software Foundation, either version 3 of the
// License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.

use anyhow::{anyhow, Result};
use chrono::{DateTime, Utc};
use quick_xml::events::Event;
use url::Url;

use crate::dated_url::DatedUrl;

use super::{FeedKind, ParsedFeed};

fn visit_rss(feed: &str, emit: &mut impl FnMut(DatedUrl) -> Result<()>) -> Result<usize> {
    let mut reader = quick_xml::Reader::from_str(feed);

    let mut buf = Vec::new();
    let mut count = 0;
    let mut depth = 0usize;
    let mut root = false;
    let mut inside_link = false;
    let mut inside_item = false;
    let mut current_link: Option<Url> = None;
    let mut current_date: Option<DateTime<Utc>> = None;

    loop {
        buf.clear();
        let event = reader.read_event_into(&mut buf);
        match &event {
            Ok(Event::Start(e)) => {
                if depth == 0 {
                    anyhow::ensure!(
                        !root && matches!(e.local_name().as_ref(), b"rss" | b"feed" | b"RDF"),
                        "invalid feed root"
                    );
                    root = true;
                }
                depth += 1;
                anyhow::ensure!(depth <= 64, "feed XML depth quota exceeded");
            }
            Ok(Event::End(_)) => {
                anyhow::ensure!(depth > 0, "unbalanced feed XML");
                depth -= 1;
            }
            Ok(Event::Empty(e)) if depth == 0 => {
                anyhow::ensure!(
                    !root && matches!(e.local_name().as_ref(), b"rss" | b"feed" | b"RDF"),
                    "invalid feed root"
                );
                root = true;
            }
            _ => {}
        }
        match event {
            Ok(Event::Start(ref e)) if e.local_name().as_ref() == b"link" => {
                inside_link = true;
            }
            Ok(Event::End(ref e)) if e.local_name().as_ref() == b"link" => {
                inside_link = false;
            }
            Ok(Event::Start(ref e)) if e.local_name().as_ref() == b"item" => {
                inside_item = true;
                current_link = None;
                current_date = None;
            }
            Ok(Event::End(ref e)) if e.local_name().as_ref() == b"item" => {
                inside_item = false;
                if let Some(link) = current_link.take() {
                    emit(DatedUrl {
                        url: link,
                        last_modified: current_date,
                    })?;
                    count += 1;
                }
            }
            Ok(Event::CData(e)) if inside_item && inside_link => {
                current_link = Url::parse(std::str::from_utf8(e.as_ref())?.trim()).ok();
            }
            Ok(Event::Text(e)) if inside_item && inside_link => {
                let link = e.unescape()?;
                if let Ok(link) = Url::parse(&link) {
                    current_link = Some(link);
                }
            }
            Ok(Event::Start(ref e))
                if e.local_name().as_ref() == b"pubDate"
                    || e.local_name().as_ref() == b"lastBuildDate" =>
            {
                if inside_item {
                    if let Ok(Event::Text(e)) = reader.read_event_into(&mut buf) {
                        if let Ok(date) = DateTime::parse_from_rfc2822(&e.unescape()?) {
                            current_date = Some(date.with_timezone(&Utc));
                        }
                    }
                }
            }
            Err(e) => {
                return Err(e.into());
            }
            Ok(Event::DocType(_)) => anyhow::bail!("feed DTD not admitted"),
            Ok(Event::Eof) => break,
            _ => {}
        }
        anyhow::ensure!(buf.len() <= 64 * 1024, "feed XML token quota exceeded");
    }

    anyhow::ensure!(root && depth == 0 && !inside_item, "incomplete RSS item");
    Ok(count)
}

fn visit_atom(feed: &str, emit: &mut impl FnMut(DatedUrl) -> Result<()>) -> Result<usize> {
    let mut reader = quick_xml::Reader::from_str(feed);

    let mut buf = Vec::new();
    let mut count = 0;
    let mut depth = 0usize;
    let mut root = false;

    let mut inside_entry = false;
    let mut current_link: Option<Url> = None;
    let mut current_date: Option<DateTime<Utc>> = None;

    loop {
        buf.clear();
        let event = reader.read_event_into(&mut buf);
        match &event {
            Ok(Event::Start(e)) => {
                if depth == 0 {
                    anyhow::ensure!(
                        !root && matches!(e.local_name().as_ref(), b"rss" | b"feed" | b"RDF"),
                        "invalid feed root"
                    );
                    root = true;
                }
                depth += 1;
                anyhow::ensure!(depth <= 64, "feed XML depth quota exceeded");
            }
            Ok(Event::End(_)) => {
                anyhow::ensure!(depth > 0, "unbalanced feed XML");
                depth -= 1;
            }
            Ok(Event::Empty(e)) if depth == 0 => {
                anyhow::ensure!(
                    !root && matches!(e.local_name().as_ref(), b"rss" | b"feed" | b"RDF"),
                    "invalid feed root"
                );
                root = true;
            }
            _ => {}
        }
        match event {
            Ok(Event::Start(ref e)) if e.local_name().as_ref() == b"entry" => {
                inside_entry = true;
                current_link = None;
                current_date = None;
            }
            Ok(Event::End(ref e)) if e.local_name().as_ref() == b"entry" => {
                inside_entry = false;
                if let Some(link) = current_link.take() {
                    emit(DatedUrl {
                        url: link,
                        last_modified: current_date,
                    })?;
                    count += 1;
                }
            }
            Ok(Event::Start(ref e)) | Ok(Event::Empty(ref e)) => {
                if inside_entry && e.local_name().as_ref() == b"link" {
                    if let Some(href) = e
                        .attributes()
                        .flatten()
                        .find(|attr| attr.key.as_ref() == b"href")
                    {
                        if let Ok(href) = href
                            .unescape_value()
                            .map_err(|e| anyhow!(e))
                            .and_then(|v| Url::parse(&v).map_err(|e| anyhow!(e)))
                        {
                            current_link = Some(href);
                        }
                    }
                } else if inside_entry
                    && (e.local_name().as_ref() == b"updated"
                        || e.local_name().as_ref() == b"published")
                {
                    if let Ok(Event::Text(e)) = reader.read_event_into(&mut buf) {
                        if let Ok(date) = DateTime::parse_from_rfc3339(&e.unescape()?) {
                            current_date = Some(date.with_timezone(&Utc));
                        }
                    }
                }
            }
            Err(e) => {
                return Err(e.into());
            }
            Ok(Event::DocType(_)) => anyhow::bail!("feed DTD not admitted"),
            Ok(Event::Eof) => break,
            _ => {}
        }
        anyhow::ensure!(buf.len() <= 64 * 1024, "feed XML token quota exceeded");
    }

    anyhow::ensure!(root && depth == 0 && !inside_entry, "incomplete Atom entry");
    Ok(count)
}

pub fn visit(
    feed: &str,
    kind: FeedKind,
    mut emit: impl FnMut(DatedUrl) -> Result<()>,
) -> Result<usize> {
    match kind {
        FeedKind::Atom => visit_atom(feed, &mut emit),
        FeedKind::Rss => visit_rss(feed, &mut emit),
    }
}
fn parse_rss(feed: &str) -> Result<ParsedFeed> {
    parse(feed, FeedKind::Rss)
}
fn parse_atom(feed: &str) -> Result<ParsedFeed> {
    parse(feed, FeedKind::Atom)
}
pub fn parse(feed: &str, kind: FeedKind) -> Result<ParsedFeed> {
    let mut links = Vec::new();
    visit(feed, kind, |link| {
        anyhow::ensure!(links.len() < 50_000, "feed URL quota exceeded");
        links.push(link);
        Ok(())
    })?;
    Ok(ParsedFeed { links })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rss() {
        let feed = r#"
        <?xml version="1.0" encoding="UTF-8"?>
        <rss xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:content="http://purl.org/rss/1.0/modules/content/" version="2.0">
            <channel>
                <title>Test title</title>
                <description>this is a description</description>
                <link>https://example.com/</link>
                <lastBuildDate>Mon, 30 Oct 2023 08:59:01 GMT</lastBuildDate>
                <item>
                    <title>First title</title>
                    <description>First desc></description>
                    <link>https://example.com/a</link>
                    <pubDate>Mon, 30 Oct 2023 08:55:00 GMT</pubDate>
                </item>
            </channel>
        </rss>
        "#;

        let parsed = parse_rss(feed).unwrap();

        assert_eq!(
            parsed.links,
            vec![DatedUrl {
                url: Url::parse("https://example.com/a").unwrap(),
                last_modified: Some(
                    DateTime::parse_from_rfc2822("Mon, 30 Oct 2023 08:55:00 GMT")
                        .unwrap()
                        .with_timezone(&Utc)
                ),
            }]
        );
    }

    #[test]
    fn test_parse_atom() {
        let feed = r#"
        <?xml version="1.0" encoding="UTF-8"?>
        <feed xmlns="http://www.w3.org/2005/Atom">
        <title>Example Feed</title>
        <entry>
            <link href="https://example.com/a"/>
            <updated>2003-12-13T18:30:02Z</updated>
        </entry>
        </feed>
        "#;

        let parsed = parse_atom(feed).unwrap();

        assert_eq!(
            parsed.links,
            vec![DatedUrl {
                url: Url::parse("https://example.com/a").unwrap(),
                last_modified: Some(
                    DateTime::parse_from_rfc3339("2003-12-13T18:30:02Z")
                        .unwrap()
                        .with_timezone(&Utc)
                ),
            }]
        );
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::*;
    #[test]
    fn prefixed_atom_cdata_rss_and_malformed_inputs_are_explicit() {
        let mut urls = Vec::new();
        visit("<atom:feed xmlns:atom='urn:atom'><atom:entry><atom:link href='https://example.org/a'/></atom:entry></atom:feed>",FeedKind::Atom,|u|{urls.push(u.url);Ok(())}).unwrap();
        assert_eq!(urls.len(), 1);
        visit("<rss><channel><item><link><![CDATA[https://example.org/b?sig=a&b=c]]></link></item></channel></rss>",FeedKind::Rss,|u|{urls.push(u.url);Ok(())}).unwrap();
        assert_eq!(urls[1].as_str(), "https://example.org/b?sig=a&b=c");
        for body in [
            "<rss><channel>",
            "<rss><item></rss>",
            "<!DOCTYPE rss><rss/>",
            "garbage",
        ] {
            assert!(visit(body, FeedKind::Rss, |_| Ok(())).is_err(), "{body}");
        }
    }
}
