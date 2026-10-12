use super::*;
use std::{
    collections::{BTreeSet, VecDeque},
    net::Ipv4Addr,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    task::JoinHandle,
};

struct ManualClock {
    now: Mutex<Instant>,
}
impl ManualClock {
    fn new() -> Self {
        Self {
            now: Mutex::new(Instant::now()),
        }
    }
    fn advance(&self, seconds: u64) {
        *self.now.lock().unwrap() += Duration::from_secs(seconds);
    }
}
impl Clock for ManualClock {
    fn now(&self) -> Instant {
        *self.now.lock().unwrap()
    }
    fn utc_now(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }
    fn sleep(&self, duration: Duration) -> IoFuture<'_, ()> {
        Box::pin(tokio::time::sleep(duration))
    }
}

enum Answer {
    Addresses(Vec<IpAddr>, Duration),
    Failed,
    Pending,
}
struct Resolver {
    answers: Mutex<VecDeque<Answer>>,
    calls: AtomicU64,
    delay: Duration,
}
impl Resolver {
    fn new(answers: Vec<Answer>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into()),
            calls: AtomicU64::new(0),
            delay: Duration::ZERO,
        })
    }
}
impl TransportResolver for Resolver {
    fn resolve<'a>(&'a self, _host: &'a str) -> IoFuture<'a, Result<DnsAnswer>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::Relaxed);
            let answer = self.answers.lock().unwrap().pop_front();
            tokio::time::sleep(self.delay).await;
            match answer.unwrap_or_else(|| public(60)) {
                Answer::Addresses(addresses, ttl) => Ok(DnsAnswer { addresses, ttl }),
                Answer::Failed => Err(Error::from(anyhow!("controlled DNS failure"))),
                Answer::Pending => futures::future::pending().await,
            }
        })
    }
}
fn public(ttl: u64) -> Answer {
    Answer::Addresses(vec!["1.1.1.1".parse().unwrap()], Duration::from_secs(ttl))
}
fn dns_transport(
    resolver: Arc<dyn TransportResolver>,
    clock: Arc<ManualClock>,
    options: TransportConfig,
) -> SharedTransport {
    SharedTransport::construct(&CrawlerConfig::for_tests(), options, resolver, clock).unwrap()
}

#[tokio::test]
async fn dns_ttls_reject_mixed_rebinding_and_do_not_extend_negative_cache_on_hits() {
    let resolver = Resolver::new(vec![
        public(2),
        Answer::Addresses(
            vec!["1.1.1.1".parse().unwrap(), "127.0.0.1".parse().unwrap()],
            Duration::from_secs(60),
        ),
        public(60),
    ]);
    let clock = Arc::new(ManualClock::new());
    let transport = dns_transport(resolver.clone(), clock.clone(), TransportConfig::default());
    let first = transport.resolve("public.test").await.unwrap();
    assert_eq!(
        transport.resolve("public.test").await.unwrap().generation,
        first.generation
    );
    clock.advance(3);
    assert!(matches!(
        transport.resolve("public.test").await,
        Err(Error::NonPublicDestination(_))
    ));
    clock.advance(4);
    assert!(transport.resolve("public.test").await.is_err());
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 2);
    clock.advance(2);
    assert_ne!(
        transport.resolve("public.test").await.unwrap().generation,
        first.generation
    );
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 3);
    assert_eq!(transport.stats().dns_rejected, 1);
    assert_eq!(transport.stats().dns_negative_hits, 1);
}

#[tokio::test]
async fn dns_failures_are_cached_and_zero_ttl_never_reuses_a_generation() {
    let resolver = Resolver::new(vec![Answer::Failed, public(0), public(0)]);
    let clock = Arc::new(ManualClock::new());
    let transport = dns_transport(resolver.clone(), clock.clone(), TransportConfig::default());
    assert!(transport.resolve("example.test").await.is_err());
    assert!(transport.resolve("example.test").await.is_err());
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    clock.advance(6);
    let before = transport.resolve("example.test").await.unwrap();
    let after = transport.resolve("example.test").await.unwrap();
    assert_ne!(before.generation, after.generation);
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 3);
}

#[tokio::test]
async fn dns_singleflight_and_entry_capacity_are_bounded() {
    let resolver = Arc::new(Resolver {
        answers: Mutex::new(VecDeque::new()),
        calls: AtomicU64::new(0),
        delay: Duration::from_millis(10),
    });
    let transport = dns_transport(
        resolver.clone(),
        Arc::new(ManualClock::new()),
        TransportConfig {
            dns_cache_entries: 2,
            dns_max_inflight: 1,
            ..Default::default()
        },
    );
    let results = futures::future::join_all((0..16).map(|_| transport.resolve("same.test"))).await;
    assert!(results.iter().all(Result::is_ok));
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    assert_eq!(transport.stats().dns_coalesced, 15);
    let hosts: Vec<_> = (0..8).map(|i| format!("origin{i}.test")).collect();
    let results = futures::future::join_all(hosts.iter().map(|host| transport.resolve(host))).await;
    assert!(results.iter().all(Result::is_ok));
    assert!(transport.stats().dns_cache_entries <= 2);
    assert_eq!(transport.stats().dns_inflight_peak, 1);
    assert_eq!(transport.stats().dns_inflight, 0);
}

#[tokio::test]
async fn cancelling_dns_leader_wakes_waiters_and_returns_its_permit() {
    let resolver = Resolver::new(vec![Answer::Pending, public(60)]);
    let transport = dns_transport(
        resolver.clone(),
        Arc::new(ManualClock::new()),
        TransportConfig::default(),
    );
    let leader = {
        let transport = transport.clone();
        tokio::spawn(async move { transport.resolve("cancel.test").await })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        while resolver.calls.load(Ordering::Relaxed) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let waiter = {
        let transport = transport.clone();
        tokio::spawn(async move { transport.resolve("cancel.test").await })
    };
    leader.abort();
    let _ = leader.await;
    assert!(tokio::time::timeout(Duration::from_secs(1), waiter)
        .await
        .unwrap()
        .unwrap()
        .is_ok());
    assert_eq!(transport.stats().dns_inflight, 0);
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 2);
}

struct LoopbackServer {
    port: u16,
    accepted: Arc<AtomicU64>,
    alive: Arc<AtomicU64>,
    hosts: Arc<Mutex<Vec<String>>>,
    listener: JoinHandle<()>,
    connections: Arc<Mutex<Vec<JoinHandle<()>>>>,
}
impl LoopbackServer {
    async fn start() -> Self {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let accepted = Arc::new(AtomicU64::new(0));
        let alive = Arc::new(AtomicU64::new(0));
        let hosts = Arc::new(Mutex::new(Vec::new()));
        let connections = Arc::new(Mutex::new(Vec::new()));
        let listener = {
            let accepted = accepted.clone();
            let alive = alive.clone();
            let hosts = hosts.clone();
            let connections = connections.clone();
            tokio::spawn(async move {
                while let Ok((stream, _)) = listener.accept().await {
                    accepted.fetch_add(1, Ordering::Relaxed);
                    alive.fetch_add(1, Ordering::Relaxed);
                    let connection = LiveConnection(alive.clone());
                    let hosts = hosts.clone();
                    connections.lock().unwrap().push(tokio::spawn(async move {
                        let _connection = connection;
                        let mut stream = BufReader::new(stream);
                        loop {
                            let mut line = String::new();
                            if !matches!(stream.read_line(&mut line).await, Ok(n) if n > 0) { return; }
                            let path = line.split_whitespace().nth(1).unwrap_or("/").to_owned();
                            loop {
                                line.clear();
                                if !matches!(stream.read_line(&mut line).await, Ok(n) if n > 0) { return; }
                                if line == "\r\n" { break; }
                                if line.to_ascii_lowercase().starts_with("host:") { hosts.lock().unwrap().push(line[5..].trim().to_owned()); }
                            }
                            if path == "/slow-headers" { tokio::time::sleep(Duration::from_millis(120)).await; }
                            if path == "/redirect" {
                                if stream.get_mut().write_all(b"HTTP/1.1 302 Found\r\nContent-Length: 0\r\nLocation: /ok\r\n\r\n").await.is_err() { return; }
                                continue;
                            }
                            if stream.get_mut().write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Type: text/html\r\nX-Transport-Fixture: preserved\r\n\r\n").await.is_err() { return; }
                            if path == "/slow-body" { tokio::time::sleep(Duration::from_millis(120)).await; }
                            if path == "/truncated" { let _ = stream.get_mut().write_all(b"hi").await; return; }
                            if stream.get_mut().write_all(b"hello").await.is_err() { return; }
                        }
                    }));
                }
            })
        };
        Self {
            port,
            accepted,
            alive,
            hosts,
            listener,
            connections,
        }
    }
    fn url(&self, host: &str, path: &str) -> Url {
        Url::parse(&format!("http://{host}:{}{path}", self.port)).unwrap()
    }
}
struct LiveConnection(Arc<AtomicU64>);
impl Drop for LiveConnection {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::Relaxed);
    }
}
impl Drop for LoopbackServer {
    fn drop(&mut self) {
        self.listener.abort();
        for task in self.connections.lock().unwrap().drain(..) {
            task.abort();
        }
    }
}
struct LoopbackResolver;
impl TransportResolver for LoopbackResolver {
    fn resolve<'a>(&'a self, _: &'a str) -> IoFuture<'a, Result<DnsAnswer>> {
        Box::pin(async {
            Ok(DnsAnswer {
                addresses: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
                ttl: Duration::from_secs(60),
            })
        })
    }
}
fn loopback_transport(options: TransportConfig) -> SharedTransport {
    let mut config = CrawlerConfig::for_tests();
    config.timeout_seconds = 2;
    SharedTransport::with_fixture_resolver(
        &config,
        options,
        Arc::new(LoopbackResolver),
        Arc::new(SystemClock),
        FixturePolicy {
            allowed_hosts: BTreeSet::from([
                "one.test".into(),
                "two.test".into(),
                "three.test".into(),
            ]),
            certificate_pem: None,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn shared_ip_budget_waits_without_taking_http_slots_and_cancellation_releases_it() {
    let server = LoopbackServer::start().await;
    let mut config = CrawlerConfig::for_tests();
    config.scheduler.max_active_per_ip = 1;
    let transport = SharedTransport::with_fixture_resolver(
        &config,
        TransportConfig {
            max_client_slots: 4,
            ..Default::default()
        },
        Arc::new(LoopbackResolver),
        Arc::new(SystemClock),
        FixturePolicy {
            allowed_hosts: BTreeSet::from(["one.test".into(), "two.test".into()]),
            certificate_pem: None,
        },
    )
    .unwrap();
    let first = transport
        .send(server.url("one.test", "/slow-body"), None)
        .await
        .unwrap();
    assert!(transport
        .send(
            server.url("two.test", "/ok"),
            Some(Duration::from_millis(40))
        )
        .await
        .is_err());
    assert_eq!(transport.stats().client_slots_active, 1);
    assert_eq!(transport.stats().ip_active_peak, 1);
    let waiting = {
        let transport = transport.clone();
        let url = server.url("two.test", "/ok");
        tokio::spawn(async move { transport.send(url, None).await })
    };
    tokio::task::yield_now().await;
    waiting.abort();
    let _ = waiting.await;
    assert_eq!(transport.stats().active_ip_reservations, 1);
    assert_eq!(first.text().await.unwrap(), "hello");
    assert_eq!(transport.stats().active_ip_reservations, 0);
    assert_eq!(
        transport
            .send(server.url("two.test", "/ok"), None)
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "hello"
    );
    assert!(transport.stats().ip_waits > 0);
}

fn loopback_body_transport(body: super::super::body::BodyConfig) -> SharedTransport {
    let mut config = CrawlerConfig::for_tests();
    config.body = body;
    config.timeout_seconds = 2;
    SharedTransport::with_fixture_resolver(
        &config,
        TransportConfig::default(),
        Arc::new(LoopbackResolver),
        Arc::new(SystemClock),
        FixturePolicy {
            allowed_hosts: BTreeSet::from(["one.test".into()]),
            certificate_pem: None,
        },
    )
    .unwrap()
}

#[tokio::test]
async fn c05_header_deadline_and_total_deadline_are_distinct_and_release_slots() {
    let server = LoopbackServer::start().await;
    let transport = loopback_body_transport(super::super::body::BodyConfig {
        header_timeout_ms: 20,
        total_timeout_ms: 1000,
        ..Default::default()
    });
    assert!(matches!(
        transport
            .send(server.url("one.test", "/slow-headers"), None)
            .await,
        Err(Error::Body(BodyError::HeadersTimeout))
    ));
    assert_eq!(transport.stats().header_timeouts, 1);
    assert_eq!(transport.stats().client_slots_active, 0);
    let transport = loopback_body_transport(super::super::body::BodyConfig {
        header_timeout_ms: 500,
        total_timeout_ms: 20,
        ..Default::default()
    });
    assert!(matches!(
        transport
            .send(
                server.url("one.test", "/slow-headers"),
                Some(Duration::from_secs(1))
            )
            .await,
        Err(Error::Body(BodyError::TotalTimeout))
    ));
    assert_eq!(transport.stats().total_timeouts, 1);
    assert_eq!(transport.stats().client_slots_active, 0);
}

#[tokio::test]
async fn c05_request_deadline_survives_headers_and_body_reader_does_not_restart_it() {
    let server = LoopbackServer::start().await;
    let body_config = super::super::body::BodyConfig {
        total_timeout_ms: 1000,
        ..Default::default()
    };
    let transport = loopback_body_transport(body_config.clone());
    let response = transport
        .send(server.url("one.test", "/ok"), Some(Duration::from_secs(10)))
        .await
        .unwrap();
    let deadline = *response.extensions().get::<RequestDeadline>().unwrap();
    assert_eq!(
        deadline.expires.duration_since(deadline.started),
        Duration::from_millis(1000)
    );
    tokio::time::sleep_until((deadline.expires + Duration::from_millis(20)).into()).await;
    let reader = super::super::body::BodyReader::new(body_config).unwrap();
    assert!(matches!(
        reader
            .read(response, super::super::body::BodyKind::Html)
            .await,
        Err(BodyError::TotalTimeout)
    ));
    assert_eq!(transport.stats().client_slots_active, 0);
    assert_eq!(reader.stats().active_bodies, 0);
}

#[tokio::test]
async fn c05_connect_timeout_covers_a_stalled_tls_handshake_on_loopback() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        futures::future::pending::<()>().await;
        drop(socket);
    });
    let transport = loopback_body_transport(super::super::body::BodyConfig {
        connect_timeout_ms: 20,
        header_timeout_ms: 500,
        total_timeout_ms: 1000,
        ..Default::default()
    });
    let result = transport
        .send(
            Url::parse(&format!("https://one.test:{port}/")).unwrap(),
            None,
        )
        .await;
    server.abort();
    assert!(
        matches!(result, Err(Error::Body(BodyError::ConnectTimeout))),
        "{result:?}"
    );
    assert_eq!(transport.stats().connect_timeouts, 1);
    assert_eq!(transport.stats().client_slots_active, 0);
}

#[tokio::test]
async fn response_metadata_host_body_and_keepalive_survive_the_lease_stream() {
    let server = LoopbackServer::start().await;
    let transport = loopback_transport(TransportConfig::default());
    let url = server.url("one.test", "/ok");
    for _ in 0..3 {
        let response = transport.send(url.clone(), None).await.unwrap();
        assert_eq!(response.url(), &url);
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        assert_eq!(response.version(), reqwest::Version::HTTP_11);
        assert_eq!(response.remote_addr().unwrap().port(), server.port);
        assert_eq!(response.headers()["x-transport-fixture"], "preserved");
        assert_eq!(response.headers()["content-length"], "5");
        assert_eq!(transport.stats().client_slots_active, 1);
        assert_eq!(response.text().await.unwrap(), "hello");
        assert_eq!(transport.stats().client_slots_active, 0);
    }
    assert_eq!(server.accepted.load(Ordering::Relaxed), 1);
    assert!(server
        .hosts
        .lock()
        .unwrap()
        .iter()
        .all(|host| host == &format!("one.test:{}", server.port)));
    assert_eq!(transport.stats().clients_created, 1);
    assert_eq!(transport.stats().clients_reused, 2);
    assert_eq!(transport.stats().bytes_received, 15);
    assert_eq!(transport.stats().dns_lookups, 1);
    let response = transport
        .send(server.url("one.test", "/redirect"), None)
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::FOUND);
    assert_eq!(response.headers()["location"], "/ok");
    drop(response);
    assert_eq!(transport.stats().client_slots_active, 0);
}

#[tokio::test]
async fn body_holds_global_and_origin_slots_and_dropping_it_releases_waiters() {
    let server = LoopbackServer::start().await;
    let transport = loopback_transport(TransportConfig {
        max_client_slots: 2,
        max_slots_per_origin: 1,
        ..Default::default()
    });
    let first = transport
        .send(server.url("one.test", "/slow-body"), None)
        .await
        .unwrap();
    let second = transport
        .send(server.url("two.test", "/slow-body"), None)
        .await
        .unwrap();
    assert_eq!(transport.stats().client_slots_active, 2);
    assert!(transport
        .send(
            server.url("one.test", "/ok"),
            Some(Duration::from_millis(15))
        )
        .await
        .is_err());
    assert!(transport
        .send(
            server.url("three.test", "/ok"),
            Some(Duration::from_millis(15))
        )
        .await
        .is_err());
    assert_eq!(server.accepted.load(Ordering::Relaxed), 2);
    drop(first);
    let third = transport
        .send(server.url("three.test", "/ok"), None)
        .await
        .unwrap();
    assert_eq!(third.text().await.unwrap(), "hello");
    drop(second);
    assert_eq!(transport.stats().client_slots_active, 0);
    assert_eq!(transport.stats().client_slots_peak, 2);
    assert_eq!(transport.stats().request_timeouts, 2);
    assert_eq!(transport.stats().bodies_dropped, 2);
    assert!(transport.stats().pool_wait_ms >= 20);
}

#[tokio::test]
async fn header_timeout_body_timeout_truncation_and_task_cancel_release_resources() {
    let server = LoopbackServer::start().await;
    let transport = loopback_transport(TransportConfig {
        max_client_slots: 1,
        max_slots_per_origin: 1,
        ..Default::default()
    });
    assert!(transport
        .send(
            server.url("one.test", "/slow-headers"),
            Some(Duration::from_millis(15))
        )
        .await
        .is_err());
    assert_eq!(transport.stats().client_slots_active, 0);
    let response = transport
        .send(
            server.url("one.test", "/slow-body"),
            Some(Duration::from_millis(20)),
        )
        .await
        .unwrap();
    assert!(response.text().await.is_err());
    assert_eq!(transport.stats().body_timeouts, 1);
    assert_eq!(transport.stats().client_slots_active, 0);
    assert!(transport
        .send(server.url("one.test", "/truncated"), None)
        .await
        .unwrap()
        .text()
        .await
        .is_err());
    assert_eq!(transport.stats().client_slots_active, 0);
    let response = transport
        .send(server.url("one.test", "/slow-body"), None)
        .await
        .unwrap();
    let task = tokio::spawn(async move { response.text().await });
    tokio::task::yield_now().await;
    task.abort();
    let _ = task.await;
    assert_eq!(transport.stats().client_slots_active, 0);
    assert_eq!(
        transport
            .send(server.url("one.test", "/ok"), None)
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "hello"
    );
    assert_eq!(transport.stats().body_failures, 2);
}

#[tokio::test]
async fn fixture_allowlist_cannot_escape_or_accept_mixed_dns_answers() {
    let transport = loopback_transport(TransportConfig::default());
    assert!(transport
        .send(Url::parse("http://outside.invalid/escape").unwrap(), None)
        .await
        .is_err());
    assert_eq!(transport.stats().dns_lookups, 0);
    let resolver = Resolver::new(vec![Answer::Addresses(
        vec!["127.0.0.1".parse().unwrap(), "1.1.1.1".parse().unwrap()],
        Duration::from_secs(60),
    )]);
    let mixed = SharedTransport::with_fixture_resolver(
        &CrawlerConfig::for_tests(),
        TransportConfig::default(),
        resolver,
        Arc::new(SystemClock),
        FixturePolicy {
            allowed_hosts: BTreeSet::from(["one.test".into()]),
            certificate_pem: None,
        },
    )
    .unwrap();
    assert!(matches!(
        mixed.resolve("one.test").await,
        Err(Error::NonPublicDestination(_))
    ));
}

#[tokio::test]
async fn native_fallback_tries_ipv6_then_ipv4_without_extra_http_attempts() {
    let server = LoopbackServer::start().await;
    let resolver = Resolver::new(vec![Answer::Addresses(
        vec!["::1".parse().unwrap(), "127.0.0.1".parse().unwrap()],
        Duration::from_secs(60),
    )]);
    let transport = SharedTransport::with_fixture_resolver(
        &CrawlerConfig::for_tests(),
        TransportConfig::default(),
        resolver.clone(),
        Arc::new(SystemClock),
        FixturePolicy {
            allowed_hosts: BTreeSet::from(["one.test".into()]),
            certificate_pem: None,
        },
    )
    .unwrap();
    // DNS prefers IPv6. The server listens only on IPv4, so native Hyper
    // fallback must use the second validated IP before sending HTTP.
    let response = transport
        .send(server.url("one.test", "/ok"), Some(Duration::from_secs(1)))
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), "hello");
    assert_eq!(server.accepted.load(Ordering::Relaxed), 1);
    assert_eq!(server.hosts.lock().unwrap().len(), 1);
    assert_eq!(transport.stats().clients_created, 1);
    assert_eq!(transport.stats().request_failures, 0);
    assert_eq!(resolver.calls.load(Ordering::Relaxed), 1);
    assert_eq!(
        transport
            .send(server.url("one.test", "/ok"), None)
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "hello"
    );
    assert_eq!(transport.stats().clients_created, 1);
    assert_eq!(transport.stats().clients_reused, 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_system_dns_retains_its_real_blocking_permit_until_lookup_finishes() {
    let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    struct ReleaseGate(Arc<(Mutex<bool>, std::sync::Condvar)>);
    impl Drop for ReleaseGate {
        fn drop(&mut self) {
            *self.0 .0.lock().unwrap() = true;
            self.0 .1.notify_all();
        }
    }
    // Release even if an assertion fails: a test must not strand a blocking worker.
    let release = ReleaseGate(gate.clone());
    let calls = Arc::new(AtomicU64::new(0));
    let mut resolver = SystemResolver::new(Duration::from_secs(60), 1);
    resolver.lookup = Some(Arc::new({
        let calls = calls.clone();
        move |_| {
            if calls.fetch_add(1, Ordering::Relaxed) == 0 {
                let (lock, wake) = &*gate;
                let mut ready = lock.lock().unwrap();
                while !*ready {
                    ready = wake.wait(ready).unwrap();
                }
            }
            Ok(vec!["1.1.1.1".parse().unwrap()])
        }
    }));
    let resolver = Arc::new(resolver);
    let first = {
        let resolver = resolver.clone();
        tokio::spawn(async move { resolver.resolve("first.test").await })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        while calls.load(Ordering::Relaxed) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    first.abort();
    let _ = first.await;
    assert_eq!(resolver.blocking_inflight.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.permits.available_permits(), 0);
    let second = {
        let resolver = resolver.clone();
        tokio::spawn(async move { resolver.resolve("second.test").await })
    };
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(
        calls.load(Ordering::Relaxed),
        1,
        "cancelled NSS still owns the only real permit"
    );
    drop(release);
    assert!(tokio::time::timeout(Duration::from_secs(1), second)
        .await
        .unwrap()
        .unwrap()
        .is_ok());
    assert_eq!(calls.load(Ordering::Relaxed), 2);
    assert_eq!(resolver.blocking_peak.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.blocking_inflight.load(Ordering::Relaxed), 0);
    assert_eq!(resolver.permits.available_permits(), 1);
}

#[tokio::test]
async fn idle_and_dns_ttl_retire_clients_and_drop_closes_retained_sockets() {
    let server = LoopbackServer::start().await;
    let clock = Arc::new(ManualClock::new());
    let transport = SharedTransport::with_fixture_resolver(
        &CrawlerConfig::for_tests(),
        TransportConfig {
            max_client_slots: 1,
            max_slots_per_origin: 1,
            idle_timeout_seconds: 2,
            dns_positive_ttl_seconds: 3,
            ..Default::default()
        },
        Arc::new(LoopbackResolver),
        clock.clone(),
        FixturePolicy {
            allowed_hosts: BTreeSet::from(["one.test".into()]),
            certificate_pem: None,
        },
    )
    .unwrap();
    let url = server.url("one.test", "/ok");
    assert_eq!(
        transport
            .send(url.clone(), None)
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "hello"
    );
    clock.advance(2);
    assert_eq!(
        transport
            .send(url.clone(), None)
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "hello"
    );
    assert_eq!(transport.stats().clients_created, 2);
    assert_eq!(transport.stats().dns_lookups, 1);
    clock.advance(1);
    assert_eq!(
        transport
            .send(url, None)
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "hello"
    );
    assert_eq!(transport.stats().clients_created, 3);
    assert_eq!(transport.stats().dns_lookups, 2);
    assert_eq!(transport.stats().client_slots_total, 1);
    assert_eq!(transport.stats().client_slots_active, 0);
    drop(transport);
    tokio::time::timeout(Duration::from_secs(1), async {
        while server.alive.load(Ordering::Relaxed) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("dropping the pool must close retained TCP connections");
}
