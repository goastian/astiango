// Stract is an open source web search engine.
// Copyright (C) 2024 Stract ApS
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

use chrono::{DateTime, Utc};
use quick_xml::events::Event;
use url::Url;

use crate::dated_url::DatedUrl;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SitemapEntry {
    Url(DatedUrl),
    Sitemap(Url),
}

pub fn visit_sitemap<R: std::io::BufRead>(
    input: R,
    mut emit: impl FnMut(SitemapEntry) -> crate::Result<()>,
) -> crate::Result<usize> {
    let mut reader = quick_xml::Reader::from_reader(input);
    let mut buffer = Vec::with_capacity(8192);
    let mut stack: Vec<Vec<u8>> = Vec::new();
    let mut root = false;
    let mut current_url = None;
    let mut current_lastmod = None;
    let mut count = 0;
    loop {
        buffer.clear();
        match reader.read_event_into(&mut buffer)? {
            Event::Start(e) => {
                let name = e.local_name().as_ref().to_vec();
                if stack.is_empty() {
                    anyhow::ensure!(
                        !root && matches!(name.as_slice(), b"urlset" | b"sitemapindex"),
                        "invalid sitemap root"
                    );
                    root = true;
                }
                anyhow::ensure!(stack.len() < 64, "sitemap XML depth quota exceeded");
                if matches!(name.as_slice(), b"url" | b"sitemap") {
                    current_url = None;
                    current_lastmod = None;
                }
                stack.push(name);
            }
            Event::Empty(e) if stack.is_empty() => {
                anyhow::ensure!(
                    !root && matches!(e.local_name().as_ref(), b"urlset" | b"sitemapindex"),
                    "invalid sitemap root"
                );
                root = true;
            }
            Event::CData(e) => {
                if stack.last().map(Vec::as_slice) == Some(b"loc") {
                    current_url = Url::parse(std::str::from_utf8(e.as_ref())?.trim())
                        .ok()
                        .filter(|u| {
                            matches!(u.scheme(), "http" | "https")
                                && u.as_str().len() <= crate::crawler::MAX_URL_LEN_BYTES
                        });
                }
            }
            Event::Text(e) => {
                let value = e.unescape()?;
                let value = value.trim();
                match stack.last().map(Vec::as_slice) {
                    Some(b"loc") => {
                        current_url = Url::parse(value).ok().filter(|url| {
                            matches!(url.scheme(), "http" | "https")
                                && url.as_str().len() <= crate::crawler::MAX_URL_LEN_BYTES
                        });
                    }
                    Some(b"lastmod") => {
                        current_lastmod = DateTime::parse_from_rfc3339(value)
                            .ok()
                            .map(|d| d.with_timezone(&Utc))
                            .or_else(|| {
                                chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                                    .ok()
                                    .and_then(|d| d.and_hms_opt(0, 0, 0))
                                    .map(|d| d.and_utc())
                            });
                    }
                    None => anyhow::ensure!(value.is_empty(), "text outside sitemap root"),
                    _ => {}
                }
            }
            Event::End(e) => {
                let name = stack
                    .pop()
                    .ok_or_else(|| anyhow::anyhow!("unbalanced sitemap XML"))?;
                anyhow::ensure!(
                    name.as_slice() == e.local_name().as_ref(),
                    "mismatched sitemap XML"
                );
                if matches!(name.as_slice(), b"url" | b"sitemap") {
                    if let Some(url) = current_url.take() {
                        let entry = if name == b"url" {
                            SitemapEntry::Url(DatedUrl {
                                url,
                                last_modified: current_lastmod.take(),
                            })
                        } else {
                            SitemapEntry::Sitemap(url)
                        };
                        emit(entry)?;
                        count += 1;
                    }
                }
            }
            Event::Eof => {
                anyhow::ensure!(root && stack.is_empty(), "incomplete sitemap XML");
                return Ok(count);
            }
            Event::DocType(_) => anyhow::bail!("sitemap DTD not admitted"),
            _ => {}
        }
        anyhow::ensure!(
            buffer.len() <= 64 * 1024,
            "sitemap XML token quota exceeded"
        );
    }
}

pub fn parse_sitemap(s: &str) -> Vec<SitemapEntry> {
    let mut entries = Vec::new();
    if let Err(error) = visit_sitemap(s.as_bytes(), |entry| {
        anyhow::ensure!(
            entries.len() < 50_000,
            "sitemap document URL quota exceeded"
        );
        entries.push(entry);
        Ok(())
    }) {
        tracing::debug!("failed to parse sitemap: {error}");
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sitemap() {
        let dr = r#"<sitemapindex>
        <sitemap>
        <loc>https://www.dr.dk/drtv/sitemap.xml</loc>
        </sitemap>
        <sitemap>
        <loc>https://www.dr.dk/sitemap.tvguide.xml</loc>
        </sitemap>
        <sitemap>
        <loc>
        https://www.dr.dk/sitemap.kommunalvalg.resultater.xml
        </loc>
        </sitemap>
        <sitemap>
        <loc>https://www.dr.dk/sitemap.folketingsvalg2022.xml</loc>
        </sitemap>
        </sitemapindex>"#;

        let entries = super::parse_sitemap(dr);
        assert_eq!(
            entries,
            vec![
                super::SitemapEntry::Sitemap("https://www.dr.dk/drtv/sitemap.xml".parse().unwrap()),
                super::SitemapEntry::Sitemap(
                    "https://www.dr.dk/sitemap.tvguide.xml".parse().unwrap()
                ),
                super::SitemapEntry::Sitemap(
                    "https://www.dr.dk/sitemap.kommunalvalg.resultater.xml"
                        .parse()
                        .unwrap()
                ),
                super::SitemapEntry::Sitemap(
                    "https://www.dr.dk/sitemap.folketingsvalg2022.xml"
                        .parse()
                        .unwrap()
                ),
            ]
        );

        let dr = r#"<urlset>
        <url>
        <lastmod>2023-10-18T05:40:04.7435930+00:00</lastmod>
        <loc>https://www.dr.dk/drtv/serie/sleepover_6382</loc>
        </url>
        <url>
        <lastmod>2023-10-18T05:40:04.7435930+00:00</lastmod>
        <loc>https://www.dr.dk/drtv/saeson/sleepover_9673</loc>
        </url>
        <url>
        <lastmod>2023-10-18T05:40:04.7435930+00:00</lastmod>
        <loc>
        https://www.dr.dk/drtv/episode/sleepover_-zoologisk-museum_52239
        </loc>
        </url>
        <url>
        <lastmod>2023-10-18T05:40:04.7435930+00:00</lastmod>
        <loc>
        https://www.dr.dk/drtv/episode/sleepover_-koebenhavns-raadhus_52252
        </loc>
        </url>
        </urlset>"#;

        let entries = super::parse_sitemap(dr);
        assert_eq!(
            entries,
            vec![
                super::SitemapEntry::Url(DatedUrl {
                    url: "https://www.dr.dk/drtv/serie/sleepover_6382"
                        .parse()
                        .unwrap(),
                    last_modified: Some(
                        "2023-10-18T05:40:04.7435930+00:00"
                            .parse::<DateTime<Utc>>()
                            .unwrap()
                    ),
                }),
                super::SitemapEntry::Url(DatedUrl {
                    url: "https://www.dr.dk/drtv/saeson/sleepover_9673"
                        .parse()
                        .unwrap(),
                    last_modified: Some(
                        "2023-10-18T05:40:04.7435930+00:00"
                            .parse::<DateTime<Utc>>()
                            .unwrap()
                    ),
                }),
                super::SitemapEntry::Url(DatedUrl {
                    url: "https://www.dr.dk/drtv/episode/sleepover_-zoologisk-museum_52239"
                        .parse()
                        .unwrap(),
                    last_modified: Some(
                        "2023-10-18T05:40:04.7435930+00:00"
                            .parse::<DateTime<Utc>>()
                            .unwrap()
                    ),
                }),
                super::SitemapEntry::Url(DatedUrl {
                    url: "https://www.dr.dk/drtv/episode/sleepover_-koebenhavns-raadhus_52252"
                        .parse()
                        .unwrap(),
                    last_modified: Some(
                        "2023-10-18T05:40:04.7435930+00:00"
                            .parse::<DateTime<Utc>>()
                            .unwrap()
                    ),
                }),
            ]
        );
    }
}

#[cfg(test)]
mod incremental_tests {
    use super::*;
    #[test]
    fn empty_cdata_and_callback_quota_are_supported_without_materializing_a_tree() {
        assert_eq!(
            visit_sitemap("<urlset/>".as_bytes(), |_| Ok(())).unwrap(),
            0
        );
        let mut url = None;
        visit_sitemap(
            "<urlset><url><loc><![CDATA[https://example.org/a?sig=x&b=y]]></loc></url></urlset>"
                .as_bytes(),
            |entry| {
                url = Some(entry);
                Ok(())
            },
        )
        .unwrap();
        assert!(matches!(url, Some(SitemapEntry::Url(_))));
        let xml="<urlset><url><loc>https://example.org/a</loc></url><url><loc>https://example.org/b</loc></url></urlset>";
        let mut count = 0;
        assert!(visit_sitemap(xml.as_bytes(), |_| {
            count += 1;
            anyhow::ensure!(count <= 1, "fixture quota");
            Ok(())
        })
        .is_err());
        assert_eq!(count, 2);
    }
}
