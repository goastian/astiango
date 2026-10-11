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
// along with this program.  If not, see <https://www.gnu.org/licenses/

use std::{
    future::Future,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use itertools::Itertools;
use tokio::sync::Mutex;
use url::Url;

use crate::{
    config::{CheckIntervals, CrawlerConfig},
    crawler,
    entrypoint::{
        indexer::IndexableWebpage,
        site_stats::{self, FinalSiteStats},
    },
    live_index::crawler::checker::{Feeds, Frontpage, Sitemap},
    webpage::url_ext::UrlExt,
    Result,
};

use super::{
    checker::{Checker, CrawlableUrl},
    crawled_db::ShardedCrawledDb,
    Client,
};

pub const MAX_PENDING_BUDGET: u64 = 32;

struct InnerCrawlableSite {
    site: site_stats::Site,
    feeds: Feeds,
    sitemap: Sitemap,
    frontpage: Frontpage,
    last_drip: Instant,
    drip_rate: Duration,
    budget: u64,
    retry_urls: Vec<CrawlableUrl>,
    retry_not_before: Option<(Instant, Duration)>,
}

impl InnerCrawlableSite {
    pub fn new(site: FinalSiteStats, client: &Client, drip_rate: Duration) -> Result<Self> {
        Ok(Self {
            site: site.site().clone(),
            feeds: Feeds::new(
                site.stats()
                    .feeds
                    .clone()
                    .into_iter()
                    .map(|feed| feed.into())
                    .collect(),
                client.reqwest(),
            ),
            sitemap: Sitemap::new(site.site(), client.reqwest())?,
            frontpage: Frontpage::new(site.site(), client.reqwest())?,
            last_drip: Instant::now(),
            drip_rate,
            budget: 0,
            retry_urls: Vec::new(),
            retry_not_before: None,
        })
    }

    pub fn drip(&mut self) {
        let pages_to_drip =
            (self.last_drip.elapsed().as_secs_f32() / self.drip_rate.as_secs_f32()) as u64;

        if pages_to_drip > 0 {
            self.last_drip = Instant::now();
            self.budget += pages_to_drip;

            if self.budget > MAX_PENDING_BUDGET {
                self.budget = MAX_PENDING_BUDGET;
            }
        }
    }
    pub fn should_crawl(&self, interval: &CheckIntervals) -> bool {
        if self
            .retry_not_before
            .is_some_and(|(started, delay)| started.elapsed() < delay)
        {
            return false;
        }
        (!self.retry_urls.is_empty()
            || self.frontpage.should_check(interval)
            || self.sitemap.should_check(interval)
            || self.feeds.should_check(interval))
            && self.budget > 0
    }
}

pub struct CrawlableSite {
    inner: Arc<Mutex<InnerCrawlableSite>>,
    currently_crawling: AtomicBool,
    site: site_stats::Site,
}

impl CrawlableSite {
    pub fn new(site: FinalSiteStats, client: &Client, drip_rate: Duration) -> Result<Self> {
        Ok(Self {
            site: site.site().clone(),
            inner: Arc::new(Mutex::new(InnerCrawlableSite::new(
                site, client, drip_rate,
            )?)),
            currently_crawling: AtomicBool::new(false),
        })
    }

    pub fn currently_crawling(&self) -> bool {
        self.currently_crawling.load(Ordering::Relaxed)
    }

    pub fn site(&self) -> &site_stats::Site {
        &self.site
    }

    pub async fn should_crawl(&self, interval: &CheckIntervals) -> bool {
        !self.currently_crawling() && self.inner.lock().await.should_crawl(interval)
    }

    pub async fn drip(&self) {
        if self.currently_crawling() {
            return;
        }

        self.inner.lock().await.drip();
    }
}

trait IndexBackend: Send + Sync {
    fn index(&self, pages: Vec<IndexableWebpage>) -> impl Future<Output = Result<()>> + Send;
}

impl IndexBackend for Client {
    async fn index(&self, pages: Vec<IndexableWebpage>) -> Result<()> {
        Client::index(self, pages).await
    }
}

struct LiveIndexSink<'a, B: IndexBackend> {
    backend: &'a B,
    pending: Mutex<Vec<crawler::CrawlDatum>>,
}

impl<'a, B: IndexBackend> LiveIndexSink<'a, B> {
    fn new(backend: &'a B) -> Self {
        Self {
            backend,
            pending: Mutex::new(Vec::new()),
        }
    }
}

impl<B: IndexBackend> crawler::DatumSink for LiveIndexSink<'_, B> {
    async fn write(&self, crawl_datum: crawler::CrawlDatum) -> Result<(), crawler::Error> {
        self.pending.lock().await.push(crawl_datum);
        Ok(())
    }

    async fn checkpoint(&self) -> Result<(), crawler::Error> {
        let mut pending = self.pending.lock().await;
        if pending.is_empty() {
            return Ok(());
        }
        self.backend
            .index(
                pending
                    .iter()
                    .cloned()
                    .map(IndexableWebpage::from)
                    .collect(),
            )
            .await?;
        pending.clear();
        Ok(())
    }

    async fn finish(&self) -> Result<(), crawler::Error> {
        self.checkpoint().await
    }
}

struct Settlement {
    retry_urls: Vec<CrawlableUrl>,
    retry_delay: Duration,
    marker_error: Option<anyhow::Error>,
}

fn settle_job(outcome: &crawler::JobOutcome, db: &ShardedCrawledDb) -> Settlement {
    let mut settlement = Settlement {
        retry_urls: Vec::new(),
        retry_delay: super::MIN_CRAWL_DELAY,
        marker_error: None,
    };
    for entry in &outcome.urls {
        match &entry.outcome {
            crawler::FetchOutcome::DurableSuccess { final_url } => {
                let stored = db
                    .insert(&final_url.clone().normalize())
                    .and_then(|()| db.insert(&entry.url.clone().normalize()));
                if let Err(error) = stored {
                    settlement.retry_urls.push(entry.url.clone().into());
                    settlement.marker_error = Some(error);
                }
            }
            crawler::FetchOutcome::Deferred { retry_after_ms, .. } => {
                settlement.retry_urls.push(entry.url.clone().into());
                if let Some(delay) = retry_after_ms {
                    settlement.retry_delay =
                        settlement.retry_delay.max(Duration::from_millis(*delay));
                }
            }
            crawler::FetchOutcome::Cancelled | crawler::FetchOutcome::StorageFailure { .. } => {
                settlement.retry_urls.push(entry.url.clone().into());
            }
            crawler::FetchOutcome::Skipped { .. }
            | crawler::FetchOutcome::PermanentFailure { .. } => {}
        }
    }
    settlement
}

pub struct CrawlableSiteGuard {
    site: Arc<CrawlableSite>,
    crawled_db: Arc<ShardedCrawledDb>,
    config: Arc<CrawlerConfig>,
}

impl CrawlableSiteGuard {
    pub async fn new(
        site: Arc<CrawlableSite>,
        crawled_db: Arc<ShardedCrawledDb>,
        config: Arc<CrawlerConfig>,
    ) -> Self {
        {
            let currently_crawling = site.currently_crawling.swap(true, Ordering::Relaxed);
            if currently_crawling {
                panic!("site is already being crawled");
            }
        }

        Self {
            site,
            crawled_db,
            config,
        }
    }

    pub fn url(&self) -> Result<Url> {
        self.site.site().url()
    }
}

impl Drop for CrawlableSiteGuard {
    fn drop(&mut self) {
        self.site.currently_crawling.store(false, Ordering::Relaxed);

        tracing::debug!(
            "Dropping crawlable site guard for site {}",
            self.site.site().as_str()
        );
    }
}

impl CrawlableSiteGuard {
    pub async fn crawl(self, client: &Client, interval: &CheckIntervals) -> Result<()> {
        let mut site = self.site.inner.lock().await;
        let mut urls = std::mem::take(&mut site.retry_urls);
        site.retry_not_before = None;

        if site.feeds.should_check(interval) {
            urls.extend(
                site.feeds
                    .get_urls_and_update_last_check()
                    .await
                    .unwrap_or_default(),
            );
        }

        if site.sitemap.should_check(interval) {
            urls.extend(
                site.sitemap
                    .get_urls_and_update_last_check()
                    .await
                    .unwrap_or_default(),
            );
        }

        if site.frontpage.should_check(interval) {
            urls.extend(
                site.frontpage
                    .get_urls_and_update_last_check()
                    .await
                    .unwrap_or_default(),
            );
        }

        let url = site.site.url()?;
        let icann_domain = url.icann_domain();
        urls = urls
            .into_iter()
            .filter(|u| u.url.icann_domain() == icann_domain)
            .unique_by(|u| u.url.clone())
            .map(|mut u| {
                u.url.normalize_in_place();
                u
            })
            .collect();

        urls.retain(|url| !self.crawled_db.has_crawled(&url.url).unwrap_or(false));

        order_urls(&mut urls);

        let budget = site.budget.min(urls.len() as u64);
        site.budget = site.budget.saturating_sub(budget);

        site.retry_urls.extend(urls.split_off(budget as usize));

        if urls.is_empty() {
            tracing::debug!("No new urls to crawl for site {}", site.site.as_str());
            return Ok(());
        }

        tracing::debug!(
            "Crawling {} urls for site {}",
            urls.len(),
            site.site.as_str()
        );

        let sink = Arc::new(LiveIndexSink::new(client));

        let executor = crawler::JobExecutor::new(
            crawler::WorkerJob {
                domain: crawler::Domain::from(site.site.url()?),
                urls: urls
                    .into_iter()
                    .take(budget as usize)
                    .map(|url| {
                        let weighted_url = crawler::WeightedUrl {
                            url: url.url,
                            weight: 1.0,
                        };

                        crawler::RetrieableUrl::from(weighted_url)
                    })
                    .collect(),
                wandering_urls: 0,
            },
            Arc::clone(&self.config),
            sink,
            client.reqwest(),
        );

        let outcome = executor.run().await;
        let settlement = settle_job(&outcome, &self.crawled_db);
        site.budget = site
            .budget
            .saturating_add(settlement.retry_urls.len() as u64)
            .min(MAX_PENDING_BUDGET);
        if !settlement.retry_urls.is_empty() {
            site.retry_not_before = Some((Instant::now(), settlement.retry_delay));
            site.retry_urls.extend(settlement.retry_urls);
        }
        if let Some(error) = settlement.marker_error {
            return Err(error);
        }
        anyhow::ensure!(
            outcome.is_success(),
            "live crawler job incomplete: {}",
            serde_json::to_string(&outcome)?
        );

        tracing::debug!("Finished crawling site {}", site.site.as_str());

        Ok(())
    }
}

fn order_urls(urls: &mut [CrawlableUrl]) {
    urls.sort_by(|a, b| match (a.last_modified, b.last_modified) {
        (Some(a), Some(b)) => a.cmp(&b).reverse(),
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    });
}

#[cfg(test)]
mod tests {
    use crate::crawler::DatumSink;
    use chrono::DateTime;
    use url::Url;

    use super::*;

    #[derive(Default)]
    struct FakeIndex {
        fail: AtomicBool,
        acknowledged: Mutex<Vec<usize>>,
    }

    impl IndexBackend for FakeIndex {
        async fn index(&self, pages: Vec<IndexableWebpage>) -> Result<()> {
            anyhow::ensure!(
                !self.fail.load(Ordering::Relaxed),
                "controlled index rejection"
            );
            self.acknowledged.lock().await.push(pages.len());
            Ok(())
        }
    }

    fn datum() -> crawler::CrawlDatum {
        crawler::CrawlDatum {
            url: Url::parse("https://example.com/page").unwrap(),
            payload_type: crate::warc::PayloadType::Html,
            body: "<html><body>live fixture</body></html>".into(),
            fetch_time_ms: 4,
            date: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn live_sink_confirms_checkpoint_and_preserves_unacknowledged_pages() {
        let backend = FakeIndex::default();
        let sink = LiveIndexSink::new(&backend);
        sink.write(datum()).await.unwrap();
        assert!(backend.acknowledged.lock().await.is_empty());
        backend.fail.store(true, Ordering::Relaxed);
        assert!(sink
            .checkpoint()
            .await
            .unwrap_err()
            .to_string()
            .contains("controlled index rejection"));
        assert_eq!(sink.pending.lock().await.len(), 1);
        backend.fail.store(false, Ordering::Relaxed);
        sink.checkpoint().await.unwrap();
        assert!(sink.pending.lock().await.is_empty());
        sink.finish().await.unwrap();
        assert_eq!(*backend.acknowledged.lock().await, [1]);
    }

    #[test]
    fn live_settlement_marks_only_durable_original_and_final_urls() {
        let directory = tempfile::tempdir().unwrap();
        let db = ShardedCrawledDb::open(directory.path()).unwrap();
        let base = Url::parse("https://example.com/").unwrap();
        let mut outcome = crawler::JobOutcome::default();
        for (path, result) in [
            (
                "original",
                crawler::FetchOutcome::DurableSuccess {
                    final_url: base.join("final").unwrap(),
                },
            ),
            (
                "later",
                crawler::FetchOutcome::Deferred {
                    reason: "http_429".into(),
                    retry_after_ms: Some(120_000),
                },
            ),
            ("cancelled", crawler::FetchOutcome::Cancelled),
            (
                "storage",
                crawler::FetchOutcome::StorageFailure {
                    message: "index rejected".into(),
                },
            ),
            (
                "missing",
                crawler::FetchOutcome::PermanentFailure {
                    reason: "http_404".into(),
                },
            ),
        ] {
            outcome.record(base.join(path).unwrap(), result);
        }
        let settlement = settle_job(&outcome, &db);
        assert!(settlement.marker_error.is_none());
        assert_eq!(
            settlement
                .retry_urls
                .iter()
                .map(|url| url.url.path())
                .collect::<Vec<_>>(),
            ["/later", "/cancelled", "/storage"]
        );
        assert_eq!(settlement.retry_delay, Duration::from_secs(120));
        for path in ["original", "final"] {
            assert!(db.has_crawled(&base.join(path).unwrap()).unwrap());
        }
        for path in ["later", "cancelled", "storage", "missing"] {
            assert!(!db.has_crawled(&base.join(path).unwrap()).unwrap());
        }
    }

    #[test]
    fn test_latest_urls_crawled_first() {
        let mut urls = vec![
            CrawlableUrl {
                url: Url::parse("https://example.com/page1").unwrap(),
                last_modified: Some(
                    DateTime::parse_from_rfc2822("Mon, 01 Jan 2024 00:00:00 GMT")
                        .unwrap()
                        .into(),
                ),
            },
            CrawlableUrl {
                url: Url::parse("https://example.com/page2").unwrap(),
                last_modified: Some(
                    DateTime::parse_from_rfc2822("Tue, 02 Jan 2024 00:00:01 GMT")
                        .unwrap()
                        .into(),
                ),
            },
            CrawlableUrl {
                url: Url::parse("https://example.com/page3").unwrap(),
                last_modified: None,
            },
        ];

        order_urls(&mut urls);

        assert_eq!(
            urls[0].url,
            Url::parse("https://example.com/page3").unwrap()
        );
        assert_eq!(
            urls[1].url,
            Url::parse("https://example.com/page2").unwrap()
        );
        assert_eq!(
            urls[2].url,
            Url::parse("https://example.com/page1").unwrap()
        );
    }
}
