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
use anyhow::anyhow;
use hashbrown::HashSet;
use itertools::Itertools;
use rand::seq::SliceRandom;

use std::{
    collections::VecDeque,
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use url::Url;

use crate::{
    config::{CrawlNetworkPolicy, CrawlerConfig},
    crawler::MAX_URL_LEN_BYTES,
    dated_url::DatedUrl,
    distributed::{retry_strategy::ExponentialBackoff, sonic},
    entrypoint::crawler::router::{NewJob, RouterService},
    sitemap::{parse_sitemap, SitemapEntry},
    webpage::url_ext::UrlExt,
};

use super::{
    body::BodyKind,
    extraction::ExtractionContext,
    outcome::{FetchOutcome, JobOutcome, JobStatus, UrlOutcome},
    robot_client::RobotClient,
    robots_txt::RobotAccess,
    wander_prioritiser::WanderPrioritiser,
    CrawlDatum, DatumSink, Domain, Error, Result, RetrieableUrl, Site, WarcWriter, WeightedUrl,
    WorkerJob,
};

const INITIAL_WANDER_STEPS: u64 = 4;

enum UrlVisit {
    Stop(FetchOutcome),
    CanCrawl,
}

struct ProcessedUrl {
    new_urls: Vec<Url>,
    final_url: Url,
}

pub struct WorkerThread {
    writer: Arc<WarcWriter>,
    config: Arc<CrawlerConfig>,
    router_hosts: Vec<SocketAddr>,
    client: RobotClient,
}

impl WorkerThread {
    pub fn new(
        writer: Arc<WarcWriter>,
        client: RobotClient,
        config: CrawlerConfig,
        router_hosts: Vec<SocketAddr>,
    ) -> Result<Self> {
        Ok(Self {
            writer,
            client,
            config: Arc::new(config),
            router_hosts,
        })
    }

    async fn router_conn(&self) -> Result<sonic::service::Connection<RouterService>> {
        let retry = ExponentialBackoff::from_millis(1_000).with_limit(Duration::from_secs(10));

        let router = *self.router_hosts.choose(&mut rand::thread_rng()).unwrap();

        sonic::service::Connection::create_with_timeout_retry(
            router,
            Duration::from_secs(90),
            retry,
        )
        .await
        .map_err(|e| Error::from(anyhow!(e)))
    }

    pub async fn run(self) -> Result<()> {
        loop {
            let mut conn = self.router_conn().await?;
            let res = conn
                .send_with_timeout(NewJob {}, Duration::from_secs(90))
                .await;

            match res {
                Ok(Some(job)) => {
                    let executor = JobExecutor::new(
                        job.into(),
                        self.config.clone(),
                        self.writer.clone(),
                        self.client.clone(),
                    );
                    let outcome = executor.run().await;
                    if outcome.status != JobStatus::Completed {
                        return Err(anyhow!(
                            "Crawler job incomplete: {}",
                            serde_json::to_string(&outcome).unwrap_or_default()
                        )
                        .into());
                    }
                    if !outcome.is_success() {
                        tracing::warn!(?outcome, "Crawler job contains permanent failures");
                    }
                }
                Ok(None) => {
                    return Ok(());
                }
                _ => {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    continue;
                }
            }
        }
    }
}

/// JobExecutor receives a job from the coordinator and crawls the urls in the job.
pub struct JobExecutor<S: DatumSink> {
    observer: Option<Arc<super::observer::Observer>>,
    cancellation: Option<Arc<AtomicBool>>,
    outcome: JobOutcome,
    telemetry_time: Duration,
    defer_remaining: Option<Option<u64>>,
    max_redirect_hops: u8,
    network_policy: CrawlNetworkPolicy,
    attempted: u64,
    attempt_limit: u64,
    seed_urls: Vec<Url>,
    writer: Arc<S>,
    client: RobotClient,
    has_gotten_429_response: bool,
    politeness_factor: u32,
    crawled_urls: HashSet<Url>,
    crawled_sitemaps: HashSet<Site>,
    sitemap_urls: HashSet<Url>,
    min_crawl_delay: Duration,
    max_crawl_delay: Duration,
    max_url_slowdown_retry: u8,
    min_politeness_factor: u32,
    max_politeness_factor: u32,
    wander_prioritiser: WanderPrioritiser,
    wandered_urls: u64,
    job: WorkerJob,
}

impl<S: DatumSink> JobExecutor<S> {
    pub fn new(
        job: WorkerJob,
        config: Arc<CrawlerConfig>,
        writer: Arc<S>,
        client: RobotClient,
    ) -> Self {
        Self {
            observer: None,
            cancellation: None,
            outcome: JobOutcome::default(),
            telemetry_time: Duration::ZERO,
            defer_remaining: None,
            max_redirect_hops: config.max_redirect_hops,
            network_policy: config.network_policy,
            attempted: 0,
            attempt_limit: job.urls.len() as u64 + job.wandering_urls,
            seed_urls: job.urls.iter().map(|url| url.url().clone()).collect(),
            writer,
            politeness_factor: config.start_politeness_factor,
            min_politeness_factor: config.min_politeness_factor,
            client,
            crawled_urls: HashSet::new(),
            crawled_sitemaps: HashSet::new(),
            wandered_urls: 0,
            sitemap_urls: HashSet::new(),
            min_crawl_delay: Duration::from_millis(config.min_crawl_delay_ms),
            max_crawl_delay: Duration::from_millis(config.max_crawl_delay_ms),
            max_url_slowdown_retry: config.max_url_slowdown_retry,
            max_politeness_factor: config.max_politeness_factor,
            wander_prioritiser: WanderPrioritiser::new(),
            has_gotten_429_response: false,
            job,
        }
    }

    pub(super) fn with_observer(mut self, observer: Arc<super::observer::Observer>) -> Self {
        self.observer = Some(observer);
        self
    }

    pub fn with_cancellation(mut self, cancellation: Arc<AtomicBool>) -> Self {
        self.cancellation = Some(cancellation);
        self
    }

    fn cancelled(&self) -> bool {
        self.cancellation
            .as_ref()
            .is_some_and(|flag| flag.load(Ordering::Acquire))
    }

    async fn cancellable<T>(
        &self,
        future: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        if self.cancelled() {
            return Err(anyhow!("cancelled").into());
        }
        match &self.cancellation {
            Some(flag) => tokio::select! {
                result = future => result,
                _ = async {
                    while !flag.load(Ordering::Acquire) { tokio::time::sleep(Duration::from_millis(100)).await; }
                } => Err(anyhow!("cancelled").into()),
            },
            None => future.await,
        }
    }

    async fn sleep(&self, duration: Duration) {
        if self.cancellation.is_none() {
            self.client.sleep(duration).await;
            return;
        }
        let mut remaining = duration;
        while !remaining.is_zero() && !self.cancelled() {
            let step = remaining.min(Duration::from_millis(100));
            self.client.sleep(step).await;
            remaining = remaining.saturating_sub(step);
        }
    }

    fn stopped(&self) -> bool {
        self.cancelled()
            || self.outcome.status == JobStatus::StorageFailed
            || self.defer_remaining.is_some()
    }

    fn record(&mut self, url: Url, outcome: FetchOutcome) {
        if matches!(outcome, FetchOutcome::DurableSuccess { .. }) {
            self.outcome.urls.push(UrlOutcome { url, outcome });
            return;
        }
        if let Some(observer) = &self.observer {
            let (kind, message) = match &outcome {
                FetchOutcome::DurableSuccess { .. } => ("fetch_completed", "Página almacenada"),
                FetchOutcome::Skipped { reason } => ("skipped", reason.as_str()),
                FetchOutcome::PermanentFailure { reason } => ("failed", reason.as_str()),
                FetchOutcome::Deferred { reason, .. } => ("deferred", reason.as_str()),
                FetchOutcome::Cancelled => ("cancelled", "Rastreo cancelado"),
                FetchOutcome::StorageFailure { message } => ("storage_failed", message.as_str()),
            };
            observer.event(&self.job.domain, Some(&url), kind, message);
        }
        self.outcome.record(url, outcome);
    }

    pub async fn run(mut self) -> JobOutcome {
        tracing::info!("Processing job: {:?}", self.job.domain);
        if let Some(observer) = &self.observer {
            observer.event(
                &self.job.domain,
                None,
                "site_started",
                "Trabajo de dominio iniciado",
            );
        }
        for site in self
            .job
            .urls
            .iter()
            .filter_map(|url| url.url().normalized_host())
        {
            if let Ok(url) = Url::parse(format!("https://{}", site).as_str()) {
                self.wander_prioritiser.inc(url, 1.0);
            }
        }

        let mut wander_steps = 0;
        while self.wandered_urls < self.job.wandering_urls
            && self.wander_prioritiser.known_urls() > 0
            && wander_steps < INITIAL_WANDER_STEPS
            && !self.stopped()
        {
            self.wander().await;
            wander_steps += 1;
        }

        self.scheduled_urls().await;

        if self.wandered_urls < self.job.wandering_urls && !self.stopped() {
            self.crawl_sitemaps().await;
        }

        while self.wandered_urls < self.job.wandering_urls
            && self.wander_prioritiser.known_urls() > 0
            && !self.stopped()
        {
            self.wander().await;
        }
        self.confirm_storage().await;
        if self.cancelled() && self.outcome.status != JobStatus::StorageFailed {
            self.outcome.status = JobStatus::Cancelled;
        }
        if let Some(observer) = &self.observer {
            let (kind, message) = match self.outcome.status {
                JobStatus::Completed => ("site_completed", "Cola del dominio resuelta"),
                JobStatus::Deferred => ("site_deferred", "Dominio con solicitudes aplazadas"),
                JobStatus::Cancelled => ("site_cancelled", "Trabajo de dominio cancelado"),
                JobStatus::StorageFailed => {
                    ("site_storage_failed", "Falló almacenamiento del dominio")
                }
            };
            observer.event(&self.job.domain, None, kind, message);
        }
        self.outcome
    }

    async fn confirm_storage(&mut self) {
        let saved: Vec<_> = self
            .outcome
            .urls
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                matches!(entry.outcome, FetchOutcome::DurableSuccess { .. }).then_some(index)
            })
            .collect();
        if saved.is_empty() {
            return;
        }
        let confirmed = {
            let _timer = super::runtime::StageTimer::optional(self.client.metrics(), "sink");
            self.writer.checkpoint().await
        };
        match confirmed {
            Ok(()) => {
                self.outcome.counters.useful_pages = saved.len() as u64;
                for index in saved {
                    if let Some(observer) = &self.observer {
                        observer.event(
                            &self.job.domain,
                            Some(&self.outcome.urls[index].url),
                            "fetch_completed",
                            "Página almacenada y confirmada",
                        );
                    }
                }
            }
            Err(error) => {
                self.outcome.status = JobStatus::StorageFailed;
                for index in saved {
                    self.outcome.urls[index].outcome = FetchOutcome::StorageFailure {
                        message: error.to_string(),
                    };
                    if let Some(observer) = &self.observer {
                        observer.event(
                            &self.job.domain,
                            Some(&self.outcome.urls[index].url),
                            "storage_failed",
                            &error.to_string(),
                        );
                    }
                }
            }
        }
    }

    async fn scheduled_urls(&mut self) {
        let urls = self.job.urls.drain(..).collect();
        self.process_urls(urls).await;
    }

    async fn wander(&mut self) {
        let mut urls: Vec<(Url, f64)> = self
            .wander_prioritiser
            .top_and_clear(self.job.wandering_urls.saturating_sub(self.wandered_urls) as usize)
            .into_iter()
            .chain(self.sitemap_urls.drain().map(|url| (url.clone(), 0.0)))
            .map(|(mut url, score)| {
                url.normalize_in_place();
                (url, score)
            })
            .filter(|(url, _)| !self.crawled_urls.contains(url))
            .filter(|(url, _)| self.job.domain == Domain::from(url))
            .filter(|(_, score)| score.is_finite())
            .unique_by(|(url, _)| url.clone())
            .collect();

        urls.sort_by(|(url_a, a), (url_b, b)| b.total_cmp(a).then_with(|| url_a.cmp(url_b)));

        let urls: VecDeque<_> = urls
            .into_iter()
            .map(|(url, _)| url)
            .map(|mut url| {
                url.normalize_in_place();
                url
            })
            .filter(|url| !self.crawled_urls.contains(url))
            .take(self.job.wandering_urls.saturating_sub(self.wandered_urls) as usize)
            .map(|url| WeightedUrl { url, weight: 0.0 })
            .map(RetrieableUrl::from)
            .collect();

        self.wandered_urls += urls.len() as u64;

        self.process_urls(urls).await;
    }

    async fn verify_url(&self, url: &Url) -> UrlVisit {
        let skipped = |reason: &str| {
            UrlVisit::Stop(FetchOutcome::Skipped {
                reason: reason.into(),
            })
        };
        if Domain::from(url) != self.job.domain {
            return skipped("outside_job_domain");
        }
        if self.crawled_urls.contains(url) {
            return skipped("already_visited");
        }
        if url.as_str().len() > MAX_URL_LEN_BYTES {
            return skipped("url_too_long");
        }
        if let Err(error) = super::robot_client::validate_network_url(self.network_policy, url) {
            return UrlVisit::Stop(self.error_outcome(error));
        }
        let access = self
            .cancellable(async { Ok(self.client.robots_txt_manager().access(url).await) })
            .await;
        match access {
            Err(_) => UrlVisit::Stop(FetchOutcome::Cancelled),
            Ok(RobotAccess::Allowed) => UrlVisit::CanCrawl,
            Ok(RobotAccess::Disallowed) => skipped("robots_disallowed"),
            Ok(RobotAccess::Unreachable) => UrlVisit::Stop(FetchOutcome::Deferred {
                reason: "robots_unavailable".into(),
                retry_after_ms: None,
            }),
        }
    }

    async fn crawl_sitemaps(&mut self) {
        let urls = self.seed_urls.clone();

        for url in urls {
            let site = Site(url.host_str().unwrap_or_default().to_string());
            if !self.crawled_sitemaps.contains(&site) {
                self.crawled_sitemaps.insert(site.clone());

                let sitemaps = self
                    .cancellable(async {
                        Ok(self.client.robots_txt_manager().sitemaps(&url).await)
                    })
                    .await;
                let sitemaps = match sitemaps {
                    Ok(value) => value,
                    Err(_) => {
                        self.record(url, FetchOutcome::Cancelled);
                        return;
                    }
                };

                for sitemap in sitemaps {
                    let res = self.urls_from_sitemap(sitemap, 5).await;
                    self.sitemap_urls
                        .extend(res.into_iter().map(|dated_url| dated_url.url));
                }
            }
        }
    }

    pub async fn process_urls(&mut self, mut urls: VecDeque<RetrieableUrl>) {
        while let Some(mut retryable_url) = urls.pop_front() {
            retryable_url.weighted_url.url.normalize_in_place();
            let url = retryable_url.url().clone();
            if self.cancelled() {
                self.record(url, FetchOutcome::Cancelled);
                continue;
            }
            if self.outcome.status == JobStatus::StorageFailed {
                self.record(
                    url,
                    FetchOutcome::Deferred {
                        reason: "storage_unavailable".into(),
                        retry_after_ms: None,
                    },
                );
                continue;
            }
            if let Some(retry_after_ms) = self.defer_remaining {
                self.record(
                    url,
                    FetchOutcome::Deferred {
                        reason: "origin_rate_limited".into(),
                        retry_after_ms,
                    },
                );
                continue;
            }
            if self.attempted >= self.attempt_limit {
                self.record(
                    url,
                    FetchOutcome::Skipped {
                        reason: "page_budget_exhausted".into(),
                    },
                );
                continue;
            }
            if let Some(observer) = &self.observer {
                observer.event(
                    &self.job.domain,
                    Some(&url),
                    "checking",
                    "Verificando robots.txt",
                );
            }
            if let UrlVisit::Stop(outcome) = self.verify_url(&url).await {
                self.record(url, outcome);
                continue;
            }
            self.attempted += 1;
            if let Some(observer) = &self.observer {
                observer.event(
                    &self.job.domain,
                    Some(&url),
                    "fetching",
                    "Descargando página",
                );
            }
            match self.process_url(url.clone()).await {
                Ok(processed) => {
                    for new_url in processed.new_urls {
                        if new_url.host_str().is_some() && Domain::from(&new_url) == self.job.domain
                        {
                            self.wander_prioritiser
                                .inc(new_url, retryable_url.weighted_url.weight);
                        }
                    }
                    self.record(
                        url,
                        FetchOutcome::DurableSuccess {
                            final_url: processed.final_url,
                        },
                    );
                }
                Err(outcome) => self.record(url, outcome),
            }
        }
    }

    fn error_outcome(&self, error: Error) -> FetchOutcome {
        if self.cancelled() {
            return FetchOutcome::Cancelled;
        }
        match error {
            Error::Body(error) if error.is_retryable() => FetchOutcome::Deferred {
                reason: error.reason().into(),
                retry_after_ms: None,
            },
            Error::Body(error) => FetchOutcome::Skipped {
                reason: error.reason().into(),
            },
            Error::DisallowedPath => FetchOutcome::Skipped {
                reason: "robots_disallowed".into(),
            },
            Error::NonPublicDestination(reason) => FetchOutcome::Skipped { reason },
            Error::RobotsUnavailable => FetchOutcome::Deferred {
                reason: "robots_unavailable".into(),
                retry_after_ms: None,
            },
            Error::FetchFailed { status_code, .. }
                if status_code.is_server_error()
                    || matches!(status_code.as_u16(), 408 | 425 | 429) =>
            {
                FetchOutcome::Deferred {
                    reason: format!("http_{}", status_code.as_u16()),
                    retry_after_ms: None,
                }
            }
            Error::ResponseBodyReadFailed | Error::Anyhow(_) => FetchOutcome::Deferred {
                reason: error.to_string(),
                retry_after_ms: None,
            },
            _ => FetchOutcome::PermanentFailure {
                reason: error.to_string(),
            },
        }
    }

    fn increase_politeness(&mut self) {
        self.politeness_factor += 1;

        if self.politeness_factor > self.max_politeness_factor {
            self.politeness_factor = self.max_politeness_factor;
        }

        tracing::warn!(
            "politeness factor increased to {} for {}",
            self.politeness_factor,
            self.job.domain.as_str()
        );
    }

    fn decrease_politeness(&mut self) {
        if self.min_politeness_factor >= self.politeness_factor {
            self.politeness_factor = self.min_politeness_factor;
            return;
        }

        self.politeness_factor = self.politeness_factor.saturating_sub(1);
    }

    async fn process_url(
        &mut self,
        mut url: Url,
    ) -> std::result::Result<ProcessedUrl, FetchOutcome> {
        let mut visited = HashSet::new();
        let mut hops = 0;
        let mut retries = 0;
        loop {
            if self.cancelled() {
                return Err(FetchOutcome::Cancelled);
            }
            if let UrlVisit::Stop(outcome) = self.verify_url(&url).await {
                return Err(outcome);
            }
            let crawl_delay = self
                .cancellable(async { Ok(self.client.robots_txt_manager().crawl_delay(&url).await) })
                .await
                .map_err(|_| FetchOutcome::Cancelled)?;
            if let Some(delay) = crawl_delay {
                self.min_crawl_delay = self.min_crawl_delay.max(delay);
                self.max_crawl_delay = self.max_crawl_delay.max(delay);
            }
            visited.insert(url.clone());
            let started = self.client.now();
            let telemetry_before = self.telemetry_time;
            let response = self.fetch_with_https_priority(url.clone()).await;
            let response = match response {
                Ok(response) => response,
                Err(error) => {
                    let elapsed = self
                        .client
                        .now()
                        .duration_since(started)
                        .saturating_sub(self.telemetry_time.saturating_sub(telemetry_before));
                    if let Some(metrics) = self.client.metrics() {
                        metrics.error(&error);
                    }
                    self.politeness_delay(elapsed).await;
                    return Err(self.error_outcome(error));
                }
            };
            let (fetched_url, response) = response;
            let status = response.status();
            let headers = response.headers().clone();
            let admitted = self
                .cancellable(self.client.body(response, BodyKind::Page))
                .await;
            let fetched = if status == reqwest::StatusCode::OK {
                admitted.map(|body| {
                    let payload_type = body
                        .payload_type()
                        .expect("page admission guarantees a WARC payload type");
                    (body, payload_type)
                })
            } else {
                Err(Error::FetchFailed {
                    status_code: status,
                    headers: headers.clone(),
                })
            };
            let elapsed = self
                .client
                .now()
                .duration_since(started)
                .saturating_sub(self.telemetry_time.saturating_sub(telemetry_before));
            if let Some(observer) = &self.observer {
                observer.response(&self.job.domain, status.as_u16());
            }
            if let Err(error) = &fetched {
                if let Some(metrics) = self.client.metrics() {
                    metrics.error(error);
                }
            }
            if matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
                self.politeness_delay(elapsed).await;
                let mut location = headers
                    .get("location")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| fetched_url.join(value).ok())
                    .ok_or_else(|| FetchOutcome::PermanentFailure {
                        reason: "invalid_redirect".into(),
                    })?;
                location.normalize_in_place();
                if visited.contains(&location) {
                    return Err(FetchOutcome::PermanentFailure {
                        reason: "redirect_cycle".into(),
                    });
                }
                if hops >= self.max_redirect_hops {
                    return Err(FetchOutcome::PermanentFailure {
                        reason: "redirect_limit".into(),
                    });
                }
                if let UrlVisit::Stop(outcome) = self.verify_url(&location).await {
                    return Err(outcome);
                }
                hops += 1;
                self.outcome.counters.redirect_hops += 1;
                if let Some(observer) = &self.observer {
                    observer.redirect(&self.job.domain, &location);
                }
                url = location;
                continue;
            }
            if status.as_u16() == 429 {
                self.has_gotten_429_response = true;
                self.increase_politeness();
                self.politeness_delay(elapsed).await;
                let retry_after = headers
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| {
                        value
                            .parse::<u64>()
                            .ok()
                            .map(Duration::from_secs)
                            .or_else(|| {
                                chrono::DateTime::parse_from_rfc2822(value)
                                    .ok()
                                    .map(|date| {
                                        Duration::from_secs(
                                            (date.with_timezone(&chrono::Utc)
                                                - self.client.utc_now())
                                            .num_seconds()
                                            .max(0)
                                                as u64,
                                        )
                                    })
                            })
                    });
                if retries >= self.max_url_slowdown_retry
                    || retry_after.is_some_and(|delay| delay > self.max_crawl_delay)
                {
                    let retry_after_ms =
                        retry_after.map(|delay| delay.as_millis().min(u64::MAX as u128) as u64);
                    self.defer_remaining = Some(retry_after_ms);
                    return Err(FetchOutcome::Deferred {
                        reason: "http_429".into(),
                        retry_after_ms,
                    });
                }
                if let Some(delay) = retry_after {
                    if let Some(observer) = &self.observer {
                        observer.waiting(&self.job.domain, delay);
                    }
                    self.sleep(delay).await;
                }
                retries += 1;
                continue;
            }
            let (body, payload_type) = match fetched {
                Ok(value) => value,
                Err(error) => {
                    self.politeness_delay(elapsed).await;
                    return Err(self.error_outcome(error));
                }
            };
            if self.cancelled() {
                return Err(FetchOutcome::Cancelled);
            }
            let (body, body_lease) = body.into_parts();
            let datum = CrawlDatum {
                url: fetched_url.clone(),
                body,
                payload_type,
                fetch_time_ms: elapsed.as_millis() as u64,
                date: self.client.utc_now(),
            };
            let extraction = {
                let _timer = super::runtime::StageTimer::optional(self.client.metrics(), "parse");
                self.client
                    .extractor()
                    .parse(
                        &datum.body,
                        &datum.url,
                        ExtractionContext {
                            domain: &self.job.domain,
                            crawled_urls: &self.crawled_urls,
                            requested_url: &url,
                        },
                    )
                    .ok()
            };
            {
                let _timer = super::runtime::StageTimer::optional(self.client.metrics(), "sink");
                self.writer
                    .write_extracted(datum, extraction.as_ref())
                    .await
                    .map_err(|error| FetchOutcome::StorageFailure {
                        message: error.to_string(),
                    })?;
            }
            self.crawled_urls.insert(url.clone());
            self.crawled_urls.insert(fetched_url.clone());
            let new_urls = extraction
                .map(|value| value.internal_urls)
                .unwrap_or_default();
            drop(body_lease);
            if !self.has_gotten_429_response {
                self.decrease_politeness();
            }
            self.politeness_delay(elapsed).await;
            return Ok(ProcessedUrl {
                new_urls,
                final_url: fetched_url,
            });
        }
    }

    async fn fetch(&mut self, url: Url) -> Result<reqwest::Response> {
        if self.cancelled() {
            return Err(anyhow!("cancelled").into());
        }
        let request = self.cancellable(self.client.get(url.clone())).await?;
        if self.cancelled() {
            return Err(anyhow!("cancelled").into());
        }
        self.outcome.counters.page_http_attempts += 1;
        if let Some(observer) = &self.observer {
            let observed_at = self.client.now();
            observer.http_attempt(&self.job.domain, &url);
            self.telemetry_time += self.client.now().duration_since(observed_at);
        }
        self.cancellable(request.send()).await
    }

    async fn fetch_with_https_priority(&mut self, url: Url) -> Result<(Url, reqwest::Response)> {
        if url.scheme() == "http" {
            let mut https = url.clone();
            https
                .set_scheme("https")
                .map_err(|_| anyhow!("set scheme on url failed"))?;
            match self.fetch(https.clone()).await {
                Ok(res) => Ok((https, res)),
                Err(
                    error @ (Error::DisallowedPath
                    | Error::RobotsUnavailable
                    | Error::NonPublicDestination(_)),
                ) => Err(error),
                Err(error) if self.cancelled() => Err(error),
                Err(_) => {
                    self.sleep(self.delay_duration()).await;
                    self.fetch(url.clone())
                        .await
                        .map(|response| (url, response))
                }
            }
        } else {
            self.fetch(url.clone())
                .await
                .map(|response| (url, response))
        }
    }

    fn delay_duration(&self) -> Duration {
        let mut delay = self.min_crawl_delay;
        delay = delay.mul_f64(2.0_f64.powi(self.politeness_factor as i32));

        if delay > self.max_crawl_delay {
            delay = self.max_crawl_delay;
        }

        delay
    }

    async fn politeness_delay(&self, fetch_time: Duration) {
        let mut delay = fetch_time;

        if delay < self.min_crawl_delay {
            delay = self.min_crawl_delay;
        }

        delay = delay.mul_f64(2.0_f64.powi(self.politeness_factor as i32));

        if delay > self.max_crawl_delay {
            delay = self.max_crawl_delay;
        }

        if let Some(observer) = &self.observer {
            observer.waiting(&self.job.domain, delay);
        }
        self.sleep(delay).await;
    }

    async fn urls_from_sitemap(&mut self, sitemap: Url, max_depth: usize) -> Vec<DatedUrl> {
        let mut stack = vec![(sitemap, 0)];
        let mut urls = vec![];

        while let Some((url, depth)) = stack.pop() {
            if self.stopped() {
                break;
            }
            if depth >= max_depth {
                continue;
            }

            let entries = async {
                let (_, response) = self.fetch_with_https_priority(url.clone()).await?;
                if response.status() != reqwest::StatusCode::OK {
                    return Err(Error::FetchFailed {
                        status_code: response.status(),
                        headers: response.headers().clone(),
                    });
                }
                let body = self
                    .cancellable(self.client.body(response, BodyKind::Sitemap))
                    .await?;
                Ok(parse_sitemap(&body))
            }
            .await;
            self.sleep(self.delay_duration()).await;
            let entries = match entries {
                Ok(entries) => entries,
                Err(error) => {
                    self.record(url, self.error_outcome(error));
                    continue;
                }
            };

            for entry in entries {
                match entry {
                    SitemapEntry::Url(url) => {
                        urls.push(url);
                    }
                    SitemapEntry::Sitemap(url) => {
                        stack.push((url, depth + 1));
                    }
                }
            }
        }

        urls
    }
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;
