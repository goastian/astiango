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

use crate::feed::Feed;
use crate::Result;

use super::{CheckIntervals, Checker, CrawlableUrl};
use crate::crawler::body::BodyKind;
use crate::crawler::robot_client::RobotClient;

pub struct Feeds {
    feeds: Vec<Feed>,
    last_check: std::time::Instant,
    client: RobotClient,
}

impl Feeds {
    pub fn new(feeds: Vec<Feed>, client: RobotClient) -> Self {
        Self {
            feeds,
            last_check: client.now(),
            client,
        }
    }
}

impl Checker for Feeds {
    async fn get_urls(&self) -> Result<Vec<CrawlableUrl>> {
        let mut urls = Vec::new();

        let config = self.client.discovery_config();
        let mut bytes = 0u64;
        let mut errors = Vec::new();
        for feed in self.feeds.iter().take(config.max_documents as usize) {
            let req = match self.client.get(feed.url.clone()).await {
                Ok(req) => req,
                Err(error) => {
                    errors.push(error.to_string());
                    continue;
                }
            };

            let resp = match req.send().await {
                Ok(resp) => resp,
                Err(error) => {
                    errors.push(error.to_string());
                    continue;
                }
            };

            let text = match self.client.body(resp, BodyKind::Feed).await {
                Ok(text) => text,
                Err(error) => {
                    errors.push(error.to_string());
                    continue;
                }
            };

            bytes = bytes.saturating_add(text.len() as u64);
            anyhow::ensure!(
                bytes <= config.max_total_bytes,
                "feed aggregate byte quota exceeded"
            );
            let candidates = match self
                .client
                .discover_document(feed.url.clone(), text, Some(feed.kind))
                .await
            {
                Ok(candidates) => candidates,
                Err(error) => {
                    errors.push(error.to_string());
                    continue;
                }
            };
            for candidate in candidates {
                if urls.len() >= config.max_urls.min(512) as usize {
                    break;
                }
                urls.push(CrawlableUrl {
                    url: candidate.url,
                    last_modified: candidate.lastmod,
                });
            }
        }

        if !errors.is_empty() {
            tracing::warn!("feed discovery partial: {}", errors.join("; "));
            anyhow::ensure!(
                !urls.is_empty(),
                "feed discovery failed: {}",
                errors.join("; ")
            );
        }
        Ok(urls)
    }

    fn next_check(&self, interval: &CheckIntervals) -> std::time::Instant {
        self.last_check + interval.feeds
    }

    fn update_last_check(&mut self) {
        self.last_check = self.client.now();
    }

    fn should_check(&self, interval: &CheckIntervals) -> bool {
        self.client.now().saturating_duration_since(self.last_check) >= interval.feeds
    }
}
