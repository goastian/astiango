use super::*;
use crate::{
    config::CrawlerConfig,
    crawler::{
        body::BodyReader,
        robot_client::RobotClient,
        runtime::{Clock, Http, IoFuture, Metrics, Runtime},
    },
    feed::{Feed, FeedKind},
};
use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

struct ImmediateClock {
    reader: Arc<Mutex<Option<BodyReader>>>,
    sleeps: Arc<AtomicUsize>,
}
impl Clock for ImmediateClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
    fn utc_now(&self) -> chrono::DateTime<Utc> {
        Utc::now()
    }
    fn sleep(&self, _: Duration) -> IoFuture<'_, ()> {
        Box::pin(async move {
            assert_eq!(
                self.reader
                    .lock()
                    .unwrap()
                    .as_ref()
                    .unwrap()
                    .stats()
                    .active_bodies,
                0,
                "courtesy must not retain a body lease"
            );
            self.sleeps.fetch_add(1, Ordering::Relaxed);
        })
    }
}
struct Fixture {
    reject: bool,
    polls: Arc<AtomicUsize>,
}
impl Http for Fixture {
    fn send(
        &self,
        url: Url,
        _: Option<Duration>,
    ) -> IoFuture<'_, std::result::Result<reqwest::Response, crate::crawler::Error>> {
        Box::pin(async move {
            let (mime, text) = match url.path() {
                "/feed" => ("application/rss+xml", "<rss><channel><item><link>https://fixture.example/page</link></item></channel></rss>"),
                "/sitemap.xml" => ("application/xml", "<urlset><url><loc>https://fixture.example/page</loc></url></urlset>"),
                _ => ("text/html", "<html><body><a href='/page'>page</a></body></html>"),
            };
            let polls = self.polls.clone();
            let stream = futures::stream::once(async move {
                polls.fetch_add(1, Ordering::Relaxed);
                Ok::<_, std::io::Error>(text.as_bytes().to_vec())
            });
            let mut response = http_02::Response::builder()
                .status(200)
                .header("content-type", mime);
            if self.reject {
                response = response.header("content-length", "4096");
            }
            Ok(response
                .body(reqwest::Body::wrap_stream(stream))
                .unwrap()
                .into())
        })
    }
}
fn client(reject: bool) -> (RobotClient, Arc<AtomicUsize>, Arc<AtomicUsize>) {
    let mut config = CrawlerConfig::for_tests();
    config.body.html.wire_limit_bytes = 1024;
    config.body.feed.wire_limit_bytes = 1024;
    config.body.sitemap.wire_limit_bytes = 1024;
    let polls = Arc::new(AtomicUsize::new(0));
    let sleeps = Arc::new(AtomicUsize::new(0));
    let reader = Arc::new(Mutex::new(None));
    let runtime = Arc::new(Runtime {
        http: Arc::new(Fixture {
            reject,
            polls: polls.clone(),
        }),
        clock: Arc::new(ImmediateClock {
            reader: reader.clone(),
            sleeps: sleeps.clone(),
        }),
        metrics: Arc::new(Metrics::default()),
    });
    let client = RobotClient::with_runtime(&config, runtime).unwrap();
    *reader.lock().unwrap() = Some(client.body_reader().clone());
    client.robots_txt_manager().insert(
        "fixture.example".into(),
        robotstxt::Robots::parse(
            "test",
            "User-agent: *\nAllow: /\nSitemap: https://fixture.example/sitemap.xml\n",
        )
        .unwrap(),
    );
    (client, polls, sleeps)
}

#[tokio::test]
async fn live_frontpage_feeds_and_sitemaps_use_admission_before_body() {
    let site =
        serde_json::from_str::<crate::entrypoint::site_stats::Site>("\"fixture.example\"").unwrap();
    for reject in [false, true] {
        let (client, polls, sleeps) = client(reject);
        let front = Frontpage::new(&site, client.clone())
            .unwrap()
            .get_urls()
            .await;
        let feeds = Feeds::new(
            vec![Feed {
                url: Url::parse("https://fixture.example/feed").unwrap(),
                kind: FeedKind::Rss,
            }],
            client.clone(),
        )
        .get_urls()
        .await
        .unwrap();
        let sitemaps = Sitemap::new(&site, client.clone())
            .unwrap()
            .get_urls()
            .await
            .unwrap();
        let stats = client.body_reader().stats();
        if reject {
            assert!(front
                .unwrap_err()
                .to_string()
                .contains("wire_limit_exceeded"));
            assert!(feeds.is_empty() && sitemaps.is_empty());
            assert_eq!(polls.load(Ordering::Relaxed), 0);
            assert_eq!(stats.rejected_content_length, 3);
        } else {
            assert_eq!(front.unwrap().len(), 1);
            assert_eq!(feeds.len(), 1);
            assert_eq!(sitemaps.len(), 1);
            assert_eq!(polls.load(Ordering::Relaxed), 3);
            assert_eq!(stats.bodies_completed, 3);
            assert_eq!(sleeps.load(Ordering::Relaxed), 2);
        }
        assert_eq!(stats.reserved_bytes, 0);
        assert_eq!(stats.active_bodies, 0);
    }
}
