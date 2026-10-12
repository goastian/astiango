//! Bounded, shared HTTP clients and fail-closed DNS for pages, robots and sitemaps.
//!
//! A slot owns one reusable reqwest client and at most one outstanding response.
//! The response body owns the slot lease, including after headers have returned.
//! DNS is checked before every checkout, and a refreshed answer invalidates the
//! previous client's pool. Host, certificate verification and SNI use the URL.

use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, SocketAddr, ToSocketAddrs},
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use anyhow::anyhow;
use futures::Stream;
use reqwest::ResponseBuilderExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{Notify, Semaphore};
use url::Url;

use super::{
    body::{BodyError, RequestDeadline},
    runtime::{Clock, IoFuture, SystemClock},
    Error, Result,
};
use crate::config::{CrawlNetworkPolicy, CrawlerConfig};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TransportConfig {
    pub max_client_slots: usize,
    pub max_slots_per_origin: usize,
    pub idle_timeout_seconds: u64,
    pub dns_cache_entries: usize,
    pub dns_positive_ttl_seconds: u64,
    pub dns_negative_ttl_seconds: u64,
    pub dns_max_inflight: usize,
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self {
            max_client_slots: 128,
            max_slots_per_origin: 4,
            idle_timeout_seconds: 30,
            dns_cache_entries: 1024,
            dns_positive_ttl_seconds: 60,
            dns_negative_ttl_seconds: 5,
            dns_max_inflight: 32,
        }
    }
}

impl TransportConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=4096).contains(&self.max_client_slots),
            "max_client_slots must be 1..4096"
        );
        anyhow::ensure!(
            self.max_slots_per_origin > 0 && self.max_slots_per_origin <= self.max_client_slots,
            "max_slots_per_origin must be 1..max_client_slots"
        );
        anyhow::ensure!(
            (1..=65536).contains(&self.dns_cache_entries),
            "dns_cache_entries must be 1..65536"
        );
        anyhow::ensure!(
            self.dns_max_inflight > 0 && self.dns_max_inflight <= self.dns_cache_entries,
            "dns_max_inflight must be 1..dns_cache_entries"
        );
        anyhow::ensure!(
            (1..=3600).contains(&self.idle_timeout_seconds)
                && (1..=3600).contains(&self.dns_positive_ttl_seconds)
                && (1..=300).contains(&self.dns_negative_ttl_seconds),
            "invalid transport TTL bounds"
        );
        Ok(())
    }
}

macro_rules! measured_counters {
    ($($name:ident),* $(,)?) => {
        #[derive(Clone, Debug, Default, Serialize)]
        pub struct TransportStats {
            $(pub $name: u64,)*
            pub max_active_per_ip: u64,
            pub active_ip_reservations: u64,
            pub max_client_slots: u64,
            pub max_slots_per_origin: u64,
            pub client_slots_total: u64,
            pub client_slots_active: u64,
            pub client_slots_idle: u64,
            pub dns_cache_entries: u64,
            /// NSS/getaddrinfo jobs that still run even when their caller cancelled.
            pub dns_blocking_inflight: u64,
            pub dns_blocking_inflight_peak: u64,
        }
        #[derive(Default)]
        struct Counters { $($name: AtomicU64,)* }
        impl Counters {
            fn snapshot(&self) -> TransportStats {
                TransportStats { $($name: self.$name.load(Ordering::Relaxed),)* ..Default::default() }
            }
        }
    }
}
measured_counters!(
    requests_started,
    headers_received,
    request_failures,
    request_timeouts,
    connect_timeouts,
    header_timeouts,
    total_timeouts,
    bodies_completed,
    body_failures,
    body_timeouts,
    bodies_dropped,
    bytes_received,
    client_slots_peak,
    clients_created,
    clients_reused,
    clients_evicted,
    pool_waits,
    pool_wait_ms,
    dns_cache_hits,
    dns_negative_hits,
    dns_cache_misses,
    dns_coalesced,
    dns_lookups,
    dns_failures,
    dns_rejected,
    dns_inflight,
    dns_inflight_peak,
    dns_lookup_ms,
    ip_waits,
    ip_active_peak
);

pub struct DnsAnswer {
    pub addresses: Vec<IpAddr>,
    pub ttl: Duration,
}

pub trait TransportResolver: Send + Sync {
    fn resolve<'a>(&'a self, host: &'a str) -> IoFuture<'a, Result<DnsAnswer>>;
}

struct SystemResolver {
    ttl: Duration,
    permits: Arc<Semaphore>,
    blocking_inflight: Arc<AtomicU64>,
    blocking_peak: Arc<AtomicU64>,
    #[cfg(test)]
    lookup: Option<Arc<dyn Fn(&str) -> std::io::Result<Vec<IpAddr>> + Send + Sync>>,
}
impl SystemResolver {
    fn new(ttl: Duration, max_inflight: usize) -> Self {
        Self {
            ttl,
            permits: Arc::new(Semaphore::new(max_inflight)),
            blocking_inflight: Arc::new(AtomicU64::new(0)),
            blocking_peak: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            lookup: None,
        }
    }
}
impl TransportResolver for SystemResolver {
    fn resolve<'a>(&'a self, host: &'a str) -> IoFuture<'a, Result<DnsAnswer>> {
        Box::pin(async move {
            // getaddrinfo exposes no authoritative record TTL. This is a bounded
            // application TTL; injected resolvers can supply a shorter record TTL.
            // Cancelling a JoinHandle cannot stop getaddrinfo/NSS. Ownership of
            // this independent permit moves into the blocking closure, so
            // repeated caller timeouts cannot create unbounded resolver threads.
            let permit = self
                .permits
                .clone()
                .acquire_owned()
                .await
                .map_err(|error| Error::from(anyhow!(error)))?;
            let host = host.to_owned();
            let inflight = self.blocking_inflight.clone();
            let peak = self.blocking_peak.clone();
            #[cfg(test)]
            let lookup = self.lookup.clone();
            let addresses = tokio::task::spawn_blocking(move || {
                let _permit = permit;
                let current = inflight.fetch_add(1, Ordering::Relaxed) + 1;
                peak.fetch_max(current, Ordering::Relaxed);
                let _running = BlockingResolution(inflight);
                #[cfg(test)]
                if let Some(lookup) = lookup {
                    return lookup(&host);
                }
                // More than 64 addresses is rejected below; retaining 65 is
                // enough to reject an excessive answer without growing a cache.
                (host.as_str(), 0)
                    .to_socket_addrs()
                    .map(|addresses| addresses.take(65).map(|address| address.ip()).collect())
            })
            .await
            .map_err(|error| Error::from(anyhow!(error)))?
            .map_err(|error| Error::from(anyhow!(error)))?;
            Ok(DnsAnswer {
                addresses,
                ttl: self.ttl,
            })
        })
    }
}

struct BlockingResolution(Arc<AtomicU64>);
impl Drop for BlockingResolution {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}

#[cfg(any(test, feature = "crawler-bench"))]
#[derive(Clone)]
pub struct FixturePolicy {
    pub allowed_hosts: std::collections::BTreeSet<String>,
    pub certificate_pem: Option<Vec<u8>>,
}

#[derive(Clone)]
struct DnsFailure {
    message: String,
    rejected: bool,
}
impl DnsFailure {
    fn error(&self) -> Error {
        if self.rejected {
            Error::NonPublicDestination(self.message.clone())
        } else {
            Error::from(anyhow!(self.message.clone()))
        }
    }
}

#[derive(Clone)]
struct Resolved {
    addresses: Arc<Vec<IpAddr>>,
    generation: u64,
}
type CachedAnswer = std::result::Result<Resolved, DnsFailure>;
struct CacheValue {
    answer: CachedAnswer,
    expires: Instant,
}
struct DnsEntry {
    value: Option<CacheValue>,
    resolving: bool,
    touched: Instant,
}
#[derive(Default)]
struct DnsState {
    entries: HashMap<String, DnsEntry>,
    generation: u64,
}

struct ClientEntry {
    client: reqwest::Client,
    generation: u64,
}
struct Slot {
    id: u64,
    origin: String,
    active: bool,
    touched: Instant,
    client: Option<ClientEntry>,
}
#[derive(Default)]
struct Pool {
    slots: Vec<Slot>,
    next_id: u64,
}

struct Inner {
    config: CrawlerConfig,
    options: TransportConfig,
    resolver: Arc<dyn TransportResolver>,
    system_resolver: Option<Arc<SystemResolver>>,
    clock: Arc<dyn Clock>,
    dns: Mutex<DnsState>,
    dns_changed: Notify,
    dns_permits: Semaphore,
    pool: Mutex<Pool>,
    pool_changed: Notify,
    ip_active: Mutex<std::collections::HashMap<IpAddr, usize>>,
    ip_changed: Notify,
    counters: Counters,
    setup_stage: Mutex<super::pipeline::BlockingStage>,
    #[cfg(any(test, feature = "crawler-bench"))]
    fixture: Option<FixturePolicy>,
}

#[derive(Clone)]
pub struct SharedTransport {
    inner: Arc<Inner>,
}

impl SharedTransport {
    pub fn new(config: &CrawlerConfig, options: TransportConfig) -> Result<Self> {
        options.validate().map_err(Error::from)?;
        let resolver = Arc::new(SystemResolver::new(
            Duration::from_secs(options.dns_positive_ttl_seconds),
            options.dns_max_inflight,
        ));
        let mut transport =
            Self::construct(config, options, resolver.clone(), Arc::new(SystemClock))?;
        Arc::get_mut(&mut transport.inner).unwrap().system_resolver = Some(resolver);
        Ok(transport)
    }

    fn construct(
        config: &CrawlerConfig,
        options: TransportConfig,
        resolver: Arc<dyn TransportResolver>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        options.validate().map_err(Error::from)?;
        config.body.validate().map_err(Error::from)?;
        config.pipeline.validate().map_err(Error::from)?;
        if config.timeout_seconds == 0 {
            return Err(Error::from(anyhow!("transport timeout must be positive")));
        }
        Ok(Self {
            inner: Arc::new(Inner {
                config: config.clone(),
                dns_permits: Semaphore::new(options.dns_max_inflight),
                options,
                resolver,
                system_resolver: None,
                clock,
                dns: Mutex::new(DnsState::default()),
                dns_changed: Notify::new(),
                pool: Mutex::new(Pool::default()),
                pool_changed: Notify::new(),
                ip_active: Mutex::new(std::collections::HashMap::new()),
                ip_changed: Notify::new(),
                counters: Counters::default(),
                setup_stage: Mutex::new(super::pipeline::BlockingStage::new(
                    config.pipeline.network_setup.clone(),
                    Duration::from_millis(config.pipeline.queue_wait_timeout_ms),
                )),
                #[cfg(any(test, feature = "crawler-bench"))]
                fixture: None,
            }),
        })
    }

    #[cfg(any(test, feature = "crawler-bench"))]
    pub fn with_fixture_resolver(
        config: &CrawlerConfig,
        options: TransportConfig,
        resolver: Arc<dyn TransportResolver>,
        clock: Arc<dyn Clock>,
        fixture: FixturePolicy,
    ) -> Result<Self> {
        if fixture.allowed_hosts.is_empty()
            || !fixture.allowed_hosts.iter().all(|host| {
                host.ends_with(".test")
                    && Url::parse(&format!("http://{host}/"))
                        .ok()
                        .and_then(|url| url.host_str().map(str::to_owned))
                        .as_deref()
                        == Some(host.as_str())
            })
        {
            return Err(Error::from(anyhow!(
                "fixture origins must be explicit .test hostnames"
            )));
        }
        let mut transport = Self::construct(config, options, resolver, clock)?;
        Arc::get_mut(&mut transport.inner).unwrap().fixture = Some(fixture);
        Ok(transport)
    }

    pub(crate) fn set_setup_stage(&self, stage: super::pipeline::BlockingStage) {
        *self.inner.setup_stage.lock().unwrap() = stage;
    }

    pub fn stats(&self) -> TransportStats {
        let mut stats = self.inner.counters.snapshot();
        stats.max_active_per_ip = self.inner.config.scheduler.max_active_per_ip as u64;
        stats.active_ip_reservations =
            self.inner.ip_active.lock().unwrap().values().sum::<usize>() as u64;
        stats.max_client_slots = self.inner.options.max_client_slots as u64;
        stats.max_slots_per_origin = self.inner.options.max_slots_per_origin as u64;
        let pool = self.inner.pool.lock().unwrap();
        stats.client_slots_total = pool.slots.len() as u64;
        stats.client_slots_active = pool.slots.iter().filter(|slot| slot.active).count() as u64;
        stats.client_slots_idle = stats.client_slots_total - stats.client_slots_active;
        drop(pool);
        stats.dns_cache_entries = self.inner.dns.lock().unwrap().entries.len() as u64;
        if let Some(resolver) = &self.inner.system_resolver {
            stats.dns_blocking_inflight = resolver.blocking_inflight.load(Ordering::Relaxed);
            stats.dns_blocking_inflight_peak = resolver.blocking_peak.load(Ordering::Relaxed);
        }
        stats
    }

    pub async fn send(&self, url: Url, timeout: Option<Duration>) -> Result<reqwest::Response> {
        self.inner
            .counters
            .requests_started
            .fetch_add(1, Ordering::Relaxed);
        let timeout = timeout
            .unwrap_or(Duration::MAX)
            .min(Duration::from_secs(self.inner.config.timeout_seconds))
            .min(Duration::from_millis(
                self.inner.config.body.total_timeout_ms,
            ));
        let started = Instant::now();
        let result = tokio::time::timeout(timeout, self.send_inner(url, timeout, started))
            .await
            .map_err(|_| {
                self.inner
                    .counters
                    .request_timeouts
                    .fetch_add(1, Ordering::Relaxed);
                self.inner
                    .counters
                    .total_timeouts
                    .fetch_add(1, Ordering::Relaxed);
                Error::Body(BodyError::TotalTimeout)
            })
            .and_then(|result| result);
        if result.is_err() {
            self.inner
                .counters
                .request_failures
                .fetch_add(1, Ordering::Relaxed);
        }
        result
    }

    fn validate_url(&self, url: &Url) -> Result<()> {
        #[cfg(any(test, feature = "crawler-bench"))]
        if let Some(fixture) = &self.inner.fixture {
            if matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
                && url
                    .host_str()
                    .is_some_and(|host| fixture.allowed_hosts.contains(host))
            {
                return Ok(());
            }
            return Err(Error::NonPublicDestination(
                "destination outside fixture allowlist".into(),
            ));
        }
        super::robot_client::validate_network_url(self.inner.config.network_policy, url)
    }

    async fn send_inner(
        &self,
        url: Url,
        timeout: Duration,
        started: Instant,
    ) -> Result<reqwest::Response> {
        self.validate_url(&url)?;
        let host = url
            .host_str()
            .ok_or_else(|| Error::from(anyhow!("missing host")))?;
        let origin = url.origin().ascii_serialization();

        let answer = match url.host() {
            Some(url::Host::Ipv4(ip)) => Resolved {
                addresses: Arc::new(vec![IpAddr::V4(ip)]),
                generation: 0,
            },
            Some(url::Host::Ipv6(ip)) => Resolved {
                addresses: Arc::new(vec![IpAddr::V6(ip)]),
                generation: 0,
            },
            _ => self.resolve(host).await?,
        };
        let ip_permit = self.acquire_ips(&answer.addresses).await;
        let mut lease = self.acquire_slot(origin).await;
        lease.ip_permit = Some(ip_permit);
        let reusable = {
            let mut pool = self.inner.pool.lock().unwrap();
            let slot = pool
                .slots
                .iter_mut()
                .find(|slot| slot.id == lease.id)
                .unwrap();
            if slot
                .client
                .as_ref()
                .is_some_and(|client| client.generation == answer.generation)
            {
                slot.client.as_ref().map(|client| client.client.clone())
            } else {
                if slot.client.take().is_some() {
                    self.inner
                        .counters
                        .clients_evicted
                        .fetch_add(1, Ordering::Relaxed);
                }
                None
            }
        };
        let client = if let Some(client) = reusable {
            self.inner
                .counters
                .clients_reused
                .fetch_add(1, Ordering::Relaxed);
            client
        } else {
            let stage = self.inner.setup_stage.lock().unwrap().clone();
            let transport = self.clone();
            let bytes = host.len().saturating_add(
                answer
                    .addresses
                    .len()
                    .saturating_mul(std::mem::size_of::<IpAddr>()),
            );
            let host = host.to_owned();
            let addresses = answer.addresses.clone();
            stage
                .run(bytes, move || transport.make_client(&host, &addresses))
                .await??
        };
        {
            let mut pool = self.inner.pool.lock().unwrap();
            pool.slots
                .iter_mut()
                .find(|slot| slot.id == lease.id)
                .unwrap()
                .client = Some(ClientEntry {
                client: client.clone(),
                generation: answer.generation,
            });
        }
        let remaining = timeout
            .checked_sub(started.elapsed())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| {
                self.inner
                    .counters
                    .request_timeouts
                    .fetch_add(1, Ordering::Relaxed);
                self.inner
                    .counters
                    .total_timeouts
                    .fetch_add(1, Ordering::Relaxed);
                Error::Body(BodyError::TotalTimeout)
            })?;
        let header_timeout = Duration::from_millis(self.inner.config.body.header_timeout_ms);
        let result = tokio::time::timeout(
            header_timeout.min(remaining),
            client.get(url).timeout(remaining).send(),
        )
        .await;
        match result {
            Ok(Ok(mut response)) => {
                if let Some(peer) = response.remote_addr() {
                    let verified = answer.addresses.contains(&peer.ip());
                    if !verified {
                        return Err(Error::NonPublicDestination(
                            "connected peer is outside verified DNS addresses".into(),
                        ));
                    }
                    let policy = match self.inner.config.network_policy {
                        CrawlNetworkPolicy::PublicOnly => "public_only",
                        CrawlNetworkPolicy::AllowPrivate => "allow_private",
                    };
                    #[cfg(any(test, feature = "crawler-bench"))]
                    let policy = if self.inner.fixture.is_some() {
                        "fixture_allowlist"
                    } else {
                        policy
                    };
                    response
                        .extensions_mut()
                        .insert(crate::capture::VerifiedPeer {
                            ip: peer.ip(),
                            validated: true,
                            policy: policy.into(),
                        });
                }
                self.inner
                    .counters
                    .headers_received
                    .fetch_add(1, Ordering::Relaxed);
                response.extensions_mut().insert(RequestDeadline {
                    started,
                    expires: started + timeout,
                });
                self.guard_response(response, lease)
            }
            Ok(Err(error)) => {
                if error.is_timeout() {
                    self.inner
                        .counters
                        .request_timeouts
                        .fetch_add(1, Ordering::Relaxed);
                    if started.elapsed() >= timeout {
                        self.inner
                            .counters
                            .total_timeouts
                            .fetch_add(1, Ordering::Relaxed);
                        return Err(Error::Body(BodyError::TotalTimeout));
                    }
                    if error.is_connect() {
                        self.inner
                            .counters
                            .connect_timeouts
                            .fetch_add(1, Ordering::Relaxed);
                        return Err(Error::Body(BodyError::ConnectTimeout));
                    }
                    self.inner
                        .counters
                        .total_timeouts
                        .fetch_add(1, Ordering::Relaxed);
                    return Err(Error::Body(BodyError::TotalTimeout));
                }
                Err(Error::from(anyhow!(error)))
            }
            Err(_) => {
                self.inner
                    .counters
                    .request_timeouts
                    .fetch_add(1, Ordering::Relaxed);
                if started.elapsed() >= timeout {
                    self.inner
                        .counters
                        .total_timeouts
                        .fetch_add(1, Ordering::Relaxed);
                    Err(Error::Body(BodyError::TotalTimeout))
                } else {
                    self.inner
                        .counters
                        .header_timeouts
                        .fetch_add(1, Ordering::Relaxed);
                    Err(Error::Body(BodyError::HeadersTimeout))
                }
            }
        }
    }

    fn make_client(&self, host: &str, addresses: &[IpAddr]) -> Result<reqwest::Client> {
        // Pin the complete validated set: Hyper retains its native IPv4/IPv6
        // fallback and Happy Eyeballs, without a second unvalidated DNS lookup.
        // A slot is a client/response lease, not a measurement of TCP sockets:
        // connection establishment can briefly race two validated addresses.
        let addresses: Vec<_> = addresses
            .iter()
            .map(|address| SocketAddr::new(*address, 0))
            .collect();
        let mut builder = super::robot_client::client_builder(&self.inner.config)
            .no_proxy()
            .connect_timeout(
                Duration::from_millis(self.inner.config.body.connect_timeout_ms)
                    .min(Duration::from_secs(self.inner.config.timeout_seconds)),
            )
            .pool_max_idle_per_host(1)
            .pool_idle_timeout(Duration::from_secs(self.inner.options.idle_timeout_seconds))
            .resolve_to_addrs(host, &addresses);
        #[cfg(any(test, feature = "crawler-bench"))]
        if let Some(pem) = self
            .inner
            .fixture
            .as_ref()
            .and_then(|fixture| fixture.certificate_pem.as_ref())
        {
            builder = builder.add_root_certificate(
                reqwest::Certificate::from_pem(pem).map_err(|error| Error::from(anyhow!(error)))?,
            );
        }
        let client = builder
            .build()
            .map_err(|error| Error::from(anyhow!(error)))?;
        self.inner
            .counters
            .clients_created
            .fetch_add(1, Ordering::Relaxed);
        Ok(client)
    }

    fn guard_response(
        &self,
        mut response: reqwest::Response,
        lease: RequestLease,
    ) -> Result<reqwest::Response> {
        let mut builder = http_02::Response::builder()
            .status(response.status())
            .version(response.version());
        // Preserve reqwest/hyper connection metadata as well as the public URL.
        *builder.extensions_mut().unwrap() = std::mem::take(response.extensions_mut());
        builder = builder.url(response.url().clone());
        *builder.headers_mut().unwrap() = response.headers().clone();
        let stream = GuardedStream {
            stream: Some(Box::pin(response.bytes_stream())),
            lease: Some(lease),
        };
        Ok(builder
            .body(reqwest::Body::wrap_stream(stream))
            .map_err(|error| Error::from(anyhow!(error)))?
            .into())
    }

    async fn acquire_ips(&self, addresses: &[IpAddr]) -> IpPermit {
        let mut addresses = addresses.to_vec();
        addresses.sort();
        addresses.dedup();
        loop {
            let notified = self.inner.ip_changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut active = self.inner.ip_active.lock().unwrap();
                if addresses.iter().all(|ip| {
                    active.get(ip).copied().unwrap_or(0)
                        < self.inner.config.scheduler.max_active_per_ip
                }) {
                    for ip in &addresses {
                        let count = active.entry(*ip).or_default();
                        *count += 1;
                        self.inner
                            .counters
                            .ip_active_peak
                            .fetch_max(*count as u64, Ordering::Relaxed);
                    }
                    return IpPermit {
                        inner: self.inner.clone(),
                        addresses,
                    };
                }
            }
            self.inner.counters.ip_waits.fetch_add(1, Ordering::Relaxed);
            notified.await;
        }
    }

    async fn acquire_slot(&self, origin: String) -> RequestLease {
        let mut wait = PoolWait {
            inner: &self.inner,
            started: Instant::now(),
            waited: false,
        };
        loop {
            let notified = self.inner.pool_changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let id = {
                let now = self.inner.clock.now();
                let mut pool = self.inner.pool.lock().unwrap();
                let ttl = Duration::from_secs(self.inner.options.idle_timeout_seconds);
                pool.slots.retain(|slot| {
                    let keep = slot.active || now.saturating_duration_since(slot.touched) < ttl;
                    if !keep && slot.client.is_some() {
                        self.inner
                            .counters
                            .clients_evicted
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    keep
                });
                if let Some(slot) = pool
                    .slots
                    .iter_mut()
                    .find(|slot| !slot.active && slot.origin == origin)
                {
                    slot.active = true;
                    Some(slot.id)
                } else if pool
                    .slots
                    .iter()
                    .filter(|slot| slot.origin == origin)
                    .count()
                    >= self.inner.options.max_slots_per_origin
                {
                    None
                } else {
                    if pool.slots.len() >= self.inner.options.max_client_slots {
                        if let Some((index, _)) = pool
                            .slots
                            .iter()
                            .enumerate()
                            .filter(|(_, slot)| !slot.active)
                            .min_by_key(|(_, slot)| slot.touched)
                        {
                            if pool.slots.remove(index).client.is_some() {
                                self.inner
                                    .counters
                                    .clients_evicted
                                    .fetch_add(1, Ordering::Relaxed);
                            }
                        }
                    }
                    if pool.slots.len() < self.inner.options.max_client_slots {
                        pool.next_id += 1;
                        let id = pool.next_id;
                        pool.slots.push(Slot {
                            id,
                            origin: origin.clone(),
                            active: true,
                            touched: now,
                            client: None,
                        });
                        self.inner
                            .counters
                            .client_slots_peak
                            .fetch_max(pool.slots.len() as u64, Ordering::Relaxed);
                        Some(id)
                    } else {
                        None
                    }
                }
            };
            if let Some(id) = id {
                return RequestLease {
                    ip_permit: None,
                    inner: self.inner.clone(),
                    id,
                    reusable: false,
                };
            }
            if !wait.waited {
                self.inner
                    .counters
                    .pool_waits
                    .fetch_add(1, Ordering::Relaxed);
                wait.waited = true;
            }
            notified.await;
        }
    }

    async fn resolve(&self, host: &str) -> Result<Resolved> {
        let mut coalesced = false;
        loop {
            let notified = self.inner.dns_changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            let lead = {
                let now = self.inner.clock.now();
                let mut state = self.inner.dns.lock().unwrap();
                if let Some(entry) = state.entries.get_mut(host) {
                    entry.touched = now;
                    if let Some(value) = &entry.value {
                        if now < value.expires {
                            match &value.answer {
                                Ok(answer) => {
                                    self.inner
                                        .counters
                                        .dns_cache_hits
                                        .fetch_add(1, Ordering::Relaxed);
                                    return Ok(answer.clone());
                                }
                                Err(error) => {
                                    self.inner
                                        .counters
                                        .dns_negative_hits
                                        .fetch_add(1, Ordering::Relaxed);
                                    return Err(error.error());
                                }
                            }
                        }
                    }
                    if entry.resolving {
                        false
                    } else {
                        entry.resolving = true;
                        true
                    }
                } else {
                    if state.entries.len() >= self.inner.options.dns_cache_entries {
                        let victim = state
                            .entries
                            .iter()
                            .filter(|(_, entry)| !entry.resolving)
                            .min_by_key(|(_, entry)| entry.touched)
                            .map(|(host, _)| host.clone());
                        if let Some(victim) = victim {
                            state.entries.remove(&victim);
                        }
                    }
                    if state.entries.len() < self.inner.options.dns_cache_entries {
                        state.entries.insert(
                            host.into(),
                            DnsEntry {
                                value: None,
                                resolving: true,
                                touched: now,
                            },
                        );
                        true
                    } else {
                        false
                    }
                }
            };
            if lead {
                self.inner
                    .counters
                    .dns_cache_misses
                    .fetch_add(1, Ordering::Relaxed);
                let mut guard = ResolutionGuard {
                    inner: self.inner.clone(),
                    host: host.into(),
                    running: false,
                    finished: false,
                };
                let _permit = self
                    .inner
                    .dns_permits
                    .acquire()
                    .await
                    .map_err(|error| Error::from(anyhow!(error)))?;
                guard.running = true;
                let inflight = self
                    .inner
                    .counters
                    .dns_inflight
                    .fetch_add(1, Ordering::Relaxed)
                    + 1;
                self.inner
                    .counters
                    .dns_inflight_peak
                    .fetch_max(inflight, Ordering::Relaxed);
                self.inner
                    .counters
                    .dns_lookups
                    .fetch_add(1, Ordering::Relaxed);
                let start = Instant::now();
                let response = self.inner.resolver.resolve(host).await;
                self.inner
                    .counters
                    .dns_lookup_ms
                    .fetch_add(start.elapsed().as_millis() as u64, Ordering::Relaxed);
                let (answer, ttl) = self.validate_answer(host, response);
                let now = self.inner.clock.now();
                let mut state = self.inner.dns.lock().unwrap();
                state.generation += 1;
                let generation = state.generation;
                let cached = answer.map(|addresses| Resolved {
                    addresses: Arc::new(addresses),
                    generation,
                });
                if let Some(entry) = state.entries.get_mut(host) {
                    entry.value = Some(CacheValue {
                        answer: cached.clone(),
                        expires: now + ttl,
                    });
                    entry.resolving = false;
                    entry.touched = now;
                }
                guard.finished = true;
                drop(state);
                self.inner.dns_changed.notify_waiters();
                return cached.map_err(|error| error.error());
            }
            if !coalesced {
                self.inner
                    .counters
                    .dns_coalesced
                    .fetch_add(1, Ordering::Relaxed);
                coalesced = true;
            }
            notified.await;
        }
    }

    fn validate_answer(
        &self,
        host: &str,
        response: Result<DnsAnswer>,
    ) -> (std::result::Result<Vec<IpAddr>, DnsFailure>, Duration) {
        let negative_ttl = Duration::from_secs(self.inner.options.dns_negative_ttl_seconds);
        let mut answer = match response {
            Ok(answer) => answer,
            Err(error) => {
                self.inner
                    .counters
                    .dns_failures
                    .fetch_add(1, Ordering::Relaxed);
                return (
                    Err(DnsFailure {
                        message: error.to_string(),
                        rejected: false,
                    }),
                    negative_ttl,
                );
            }
        };
        let public = self.inner.config.network_policy == CrawlNetworkPolicy::PublicOnly;
        let mut accepted = !answer.addresses.is_empty()
            && answer.addresses.len() <= 64
            && (!public
                || answer
                    .addresses
                    .iter()
                    .all(|ip| super::robot_client::is_public_address(*ip)));
        #[cfg(any(test, feature = "crawler-bench"))]
        if self.inner.fixture.is_some() {
            accepted = !answer.addresses.is_empty()
                && answer.addresses.len() <= 64
                && answer.addresses.iter().all(IpAddr::is_loopback);
        }
        if !accepted {
            self.inner
                .counters
                .dns_rejected
                .fetch_add(1, Ordering::Relaxed);
            return (
                Err(DnsFailure {
                    message: format!(
                        "DNS returned an empty, excessive or disallowed address set for {host}"
                    ),
                    rejected: true,
                }),
                negative_ttl,
            );
        }
        // Preserve the resolver's address-family preference while deduplicating.
        let mut seen = HashSet::new();
        answer.addresses.retain(|address| seen.insert(*address));
        (
            Ok(answer.addresses),
            answer.ttl.min(Duration::from_secs(
                self.inner.options.dns_positive_ttl_seconds,
            )),
        )
    }
}

struct ResolutionGuard {
    inner: Arc<Inner>,
    host: String,
    running: bool,
    finished: bool,
}
impl Drop for ResolutionGuard {
    fn drop(&mut self) {
        if self.running {
            self.inner
                .counters
                .dns_inflight
                .fetch_sub(1, Ordering::Relaxed);
        }
        if !self.finished {
            if let Some(entry) = self.inner.dns.lock().unwrap().entries.get_mut(&self.host) {
                entry.resolving = false;
            }
        }
        self.inner.dns_changed.notify_waiters();
    }
}

struct IpPermit {
    inner: Arc<Inner>,
    addresses: Vec<IpAddr>,
}
impl Drop for IpPermit {
    fn drop(&mut self) {
        let mut active = self.inner.ip_active.lock().unwrap();
        for ip in &self.addresses {
            if let Some(count) = active.get_mut(ip) {
                *count -= 1;
                if *count == 0 {
                    active.remove(ip);
                }
            }
        }
        drop(active);
        self.inner.ip_changed.notify_waiters();
    }
}

struct RequestLease {
    ip_permit: Option<IpPermit>,
    inner: Arc<Inner>,
    id: u64,
    reusable: bool,
}
impl Drop for RequestLease {
    fn drop(&mut self) {
        let mut pool = self.inner.pool.lock().unwrap();
        if let Some(slot) = pool.slots.iter_mut().find(|slot| slot.id == self.id) {
            if !self.reusable && slot.client.take().is_some() {
                self.inner
                    .counters
                    .clients_evicted
                    .fetch_add(1, Ordering::Relaxed);
            }
            slot.active = false;
            slot.touched = self.inner.clock.now();
        }
        drop(pool);
        self.inner.pool_changed.notify_waiters();
    }
}

struct PoolWait<'a> {
    inner: &'a Inner,
    started: Instant,
    waited: bool,
}
impl Drop for PoolWait<'_> {
    fn drop(&mut self) {
        if self.waited {
            self.inner
                .counters
                .pool_wait_ms
                .fetch_add(self.started.elapsed().as_millis() as u64, Ordering::Relaxed);
        }
    }
}

struct GuardedStream<S> {
    stream: Option<Pin<Box<S>>>,
    lease: Option<RequestLease>,
}
impl<S, B> Stream for GuardedStream<S>
where
    S: Stream<Item = std::result::Result<B, reqwest::Error>>,
    B: AsRef<[u8]>,
{
    type Item = std::result::Result<B, reqwest::Error>;
    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        let Some(stream) = &mut this.stream else {
            return Poll::Ready(None);
        };
        match stream.as_mut().poll_next(cx) {
            Poll::Ready(Some(Ok(bytes))) => {
                if let Some(lease) = &this.lease {
                    lease
                        .inner
                        .counters
                        .bytes_received
                        .fetch_add(bytes.as_ref().len() as u64, Ordering::Relaxed);
                }
                Poll::Ready(Some(Ok(bytes)))
            }
            Poll::Ready(Some(Err(error))) => {
                if let Some(lease) = &this.lease {
                    lease
                        .inner
                        .counters
                        .body_failures
                        .fetch_add(1, Ordering::Relaxed);
                    if error.is_timeout() {
                        lease
                            .inner
                            .counters
                            .body_timeouts
                            .fetch_add(1, Ordering::Relaxed);
                    }
                }
                this.stream.take();
                this.lease.take();
                Poll::Ready(Some(Err(error)))
            }
            Poll::Ready(None) => {
                if let Some(lease) = &mut this.lease {
                    lease.reusable = true;
                    lease
                        .inner
                        .counters
                        .bodies_completed
                        .fetch_add(1, Ordering::Relaxed);
                }
                this.stream.take();
                this.lease.take();
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}
impl<S> Drop for GuardedStream<S> {
    fn drop(&mut self) {
        if let Some(lease) = &self.lease {
            lease
                .inner
                .counters
                .bodies_dropped
                .fetch_add(1, Ordering::Relaxed);
        }
        // Cancel/drop the real response before making its slot available again.
        self.stream.take();
        self.lease.take();
    }
}

#[cfg(test)]
mod tests {
    include!("transport_tests.rs");
}
