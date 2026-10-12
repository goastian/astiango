use super::*;
use crate::crawler::{
    robot_client::RobotClient,
    runtime::{IoFuture, SystemClock},
    transport::{DnsAnswer, FixturePolicy, TransportResolver},
};
use std::{
    collections::BTreeSet,
    net::IpAddr,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

struct LoopbackDns;

impl TransportResolver for LoopbackDns {
    fn resolve<'a>(&'a self, _: &'a str) -> IoFuture<'a, crate::crawler::Result<DnsAnswer>> {
        Box::pin(async {
            Ok(DnsAnswer {
                addresses: vec![IpAddr::from([127, 0, 0, 1])],
                ttl: Duration::from_secs(60),
            })
        })
    }
}

#[tokio::test]
async fn robots_pages_and_client_clones_share_pool_without_cross_origin_reuse() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let connections = Arc::new(AtomicUsize::new(0));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let server_connections = connections.clone();
    let server_requests = requests.clone();
    let server = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                break;
            };
            let id = server_connections.fetch_add(1, Ordering::SeqCst);
            let requests = server_requests.clone();
            tokio::spawn(async move {
                loop {
                    let mut header = Vec::new();
                    while !header.ends_with(b"\r\n\r\n") {
                        let mut byte = [0u8];
                        if socket.read_exact(&mut byte).await.is_err() || header.len() >= 8192 {
                            return;
                        }
                        header.push(byte[0]);
                    }
                    let header = String::from_utf8(header).unwrap();
                    let path = header
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap()
                        .to_owned();
                    let host = header
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("host")
                                .then(|| value.trim().to_owned())
                        })
                        .unwrap();
                    requests.lock().unwrap().push((id, host, path.clone()));
                    let body = if path == "/robots.txt" {
                        "User-agent: *\nDisallow: /blocked\n"
                    } else {
                        "<html><body>controlled page</body></html>"
                    };
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: text/html\r\n\r\n{}",
                        body.len(), body
                    );
                    if socket.write_all(response.as_bytes()).await.is_err() {
                        return;
                    }
                }
            });
        }
    });

    let mut config = CrawlerConfig::for_tests();
    config.network_policy = CrawlNetworkPolicy::AllowPrivate;
    config.min_crawl_delay_ms = 0;
    let transport = SharedTransport::with_fixture_resolver(
        &config,
        config.transport.clone(),
        Arc::new(LoopbackDns),
        Arc::new(SystemClock),
        FixturePolicy {
            allowed_hosts: BTreeSet::from([
                "first.integration.test".into(),
                "second.integration.test".into(),
            ]),
            certificate_pem: None,
        },
    )
    .unwrap();
    let client = RobotClient::with_transport(&config, transport.clone());
    let first = Url::parse(&format!("http://first.integration.test:{port}/")).unwrap();
    let second = Url::parse(&format!("http://second.integration.test:{port}/")).unwrap();
    let response = client
        .robots_txt_manager()
        .inner
        .request(first.join("robots.txt").unwrap())
        .await
        .unwrap()
        .send()
        .await
        .unwrap();
    let robots = response.text().await.unwrap();
    for origin in [
        first.origin().ascii_serialization(),
        second.origin().ascii_serialization(),
    ] {
        client.robots_txt_manager().insert(
            origin,
            robotstxt::Robots::parse(&config.user_agent.token, &robots).unwrap(),
        );
    }
    for (copy, url) in [
        (client.clone(), first.join("one").unwrap()),
        (client.clone(), first.join("two").unwrap()),
        (client.clone(), second.join("three").unwrap()),
    ] {
        let response = copy.get(url.clone()).await.unwrap().send().await.unwrap();
        assert_eq!(response.url(), &url);
        assert!(response.text().await.unwrap().contains("controlled page"));
    }
    assert!(matches!(
        client.get(first.join("blocked").unwrap()).await,
        Err(crate::crawler::Error::DisallowedPath)
    ));

    let stats = transport.stats();
    assert_eq!(stats.requests_started, 4);
    assert_eq!(stats.bodies_completed, 4);
    assert_eq!(stats.dns_lookups, 2);
    assert_eq!(stats.clients_created, 2);
    assert_eq!(stats.clients_reused, 2);
    assert_eq!(stats.client_slots_active, 0);
    assert_eq!(connections.load(Ordering::SeqCst), 2);
    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[..3]
        .iter()
        .all(|(id, host, _)| *id == requests[0].0
            && host == &format!("first.integration.test:{port}")));
    assert_ne!(requests[0].0, requests[3].0);
    assert_eq!(requests[3].1, format!("second.integration.test:{port}"));
    server.abort();
}
