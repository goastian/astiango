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

use url::Url;

use crate::crawler::body::BodyKind;
use crate::crawler::robot_client::RobotClient;
use crate::Result;
use crate::{entrypoint::site_stats, webpage::url_ext::UrlExt};

use super::{CheckIntervals, Checker, CrawlableUrl};

pub struct Sitemap {
    robots_url: Url,
    last_check: std::time::Instant,
    client: RobotClient,
}

impl Sitemap {
    pub fn new(site: &site_stats::Site, client: RobotClient) -> Result<Self> {
        let robots_url = Url::robust_parse(&format!("{}/robots.txt", site.as_str()))?;

        Ok(Self {
            robots_url,
            last_check: client.now(),
            client,
        })
    }

    async fn sitemap_urls(&self) -> Vec<Url> {
        self.client
            .robots_txt_manager()
            .sitemaps(&self.robots_url)
            .await
    }

    async fn cycle_urls(&self, sitemaps: Vec<Url>) -> Result<Vec<CrawlableUrl>> {
        let config = self.client.discovery_config();
        let mut stack = sitemaps
            .into_iter()
            .take(config.max_documents as usize)
            .map(|url| (url, 0u32))
            .collect::<std::collections::VecDeque<_>>();
        let mut visited = std::collections::HashSet::new();
        let mut urls = Vec::new();
        let mut bytes = 0u64;
        while let Some((url, depth)) = stack.pop_front() {
            if depth >= config.max_depth || visited.contains(&url) {
                continue;
            }
            anyhow::ensure!(
                visited.len() < config.max_documents as usize,
                "sitemap aggregate document quota exceeded"
            );
            visited.insert(url.clone());
            let response = self.client.get(url.clone()).await?.send().await?;
            let body = self.client.body(response, BodyKind::Sitemap).await?;
            bytes = bytes.saturating_add(body.len() as u64);
            anyhow::ensure!(
                bytes <= config.max_total_bytes,
                "sitemap aggregate byte quota exceeded"
            );
            for candidate in self.client.discover_document(url, body, None).await? {
                if candidate.sitemap {
                    if !visited.contains(&candidate.url) {
                        anyhow::ensure!(
                            stack.len() + visited.len() < config.max_documents as usize,
                            "sitemap branching quota exceeded"
                        );
                        stack.push_back((candidate.url, depth + 1));
                    }
                } else {
                    anyhow::ensure!(
                        urls.len() < config.max_urls.min(512) as usize,
                        "live sitemap candidate quota exceeded; candidates retained in frontier"
                    );
                    urls.push(CrawlableUrl {
                        url: candidate.url,
                        last_modified: candidate.lastmod,
                    });
                }
            }
        }
        Ok(urls)
    }
}

impl Checker for Sitemap {
    async fn get_urls(&self) -> Result<Vec<CrawlableUrl>> {
        self.cycle_urls(self.sitemap_urls().await).await
    }

    fn next_check(&self, interval: &CheckIntervals) -> std::time::Instant {
        self.last_check + interval.sitemap
    }

    fn update_last_check(&mut self) {
        self.last_check = self.client.now();
    }

    fn should_check(&self, interval: &CheckIntervals) -> bool {
        self.client.now().saturating_duration_since(self.last_check) >= interval.sitemap
    }
}
