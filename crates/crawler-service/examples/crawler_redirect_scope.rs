//! Real HTTP integration against two owned loopback sites. Never crawls outsiders.
use anyhow::Result;
use astiango_crawler_service::{config::{CrawlerConfig, CrawlNetworkPolicy}, crawler::{
    JobExecutor, robot_client::RobotClient, CrawlDatum, DatumSink, Domain, FetchOutcome, Job, WeightedUrl,
}};
use std::{collections::VecDeque, io::{Read, Write}, net::TcpListener,
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, thread, time::Duration};
use url::Url;

struct Server { url: Url, stop: Arc<AtomicBool>, thread: Option<thread::JoinHandle<()>>, requests: Arc<Mutex<Vec<String>>> }
impl Server {
    fn start(address: &str, target: Option<Url>) -> Result<Self> {
        let listener = TcpListener::bind((address, 0))?;
        listener.set_nonblocking(true)?;
        let url = Url::parse(&format!("http://{}/", listener.local_addr()?))?;
        let stop = Arc::new(AtomicBool::new(false)); let stopped = stop.clone();
        let requests = Arc::new(Mutex::new(Vec::new())); let seen = requests.clone();
        let thread = thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(pair) => pair, Err(error) if error.kind()==std::io::ErrorKind::WouldBlock => {thread::sleep(Duration::from_millis(5)); continue;}, Err(_) => break,
                };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut request = Vec::new(); let mut buffer = [0; 1024];
                while request.len() < 8192 && !request.windows(4).any(|v|v==b"\r\n\r\n") {
                    match stream.read(&mut buffer) { Ok(0)|Err(_) => break, Ok(n) => request.extend_from_slice(&buffer[..n]) }
                }
                let text = String::from_utf8_lossy(&request);
                let path = text.lines().next().and_then(|line|line.split_whitespace().nth(1)).unwrap_or("/");
                seen.lock().unwrap().push(path.to_string());
                let (status, headers, body) = if path=="/robots.txt" { ("404 Not Found",String::new(),"") }
                    else if let Some(target)=&target { ("302 Found",format!("Location: {target}\r\n"),"") }
                    else { ("200 OK","Content-Type: text/html\r\n".to_string(),"<html><head><title>Owned redirect destination</title></head><body>A real local response after a cross-site redirect.</body></html>") };
                let _ = write!(stream,"HTTP/1.1 {status}\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
            }
        });
        Ok(Self { url, stop, thread:Some(thread), requests })
    }
}
impl Drop for Server { fn drop(&mut self) {self.stop.store(true,Ordering::Relaxed);if let Some(thread)=self.thread.take(){let _=thread.join();}} }
#[derive(Default)] struct Sink(Mutex<Vec<CrawlDatum>>);
impl DatumSink for Sink {
    async fn write(&self, datum:CrawlDatum)->std::result::Result<(),astiango_crawler_service::crawler::Error> {self.0.lock().unwrap().push(datum);Ok(())}
    async fn finish(&self)->std::result::Result<(),astiango_crawler_service::crawler::Error> {Ok(())}
}

fn main()->Result<()> {
    let destination = Server::start("127.0.0.2", None)?;
    let source = Server::start("127.0.0.1", Some(destination.url.clone()))?;
    let mut config: CrawlerConfig = serde_json::from_value(serde_json::json!({
        "num_worker_threads":1,"timeout_seconds":5,"user_agent":{"full":"AstianGO-Local-Fixture/1.0","token":"AstianGOLocalFixture"},
        "s3":{"bucket":"","folder":"","access_key":"","secret_key":"","endpoint":""},"router_hosts":[]
    }))?;
    config.network_policy = CrawlNetworkPolicy::AllowPrivate; // This fixture only.
    config.min_crawl_delay_ms=0;config.start_politeness_factor=0;config.min_politeness_factor=0;
    let sink=Arc::new(Sink::default());
    let job = || Job { domain:Domain::from(source.url.clone()),
        urls:VecDeque::from([WeightedUrl{url:source.url.clone(),weight:1.0}]),wandering_urls:0 };
    let rt=tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
    let outcome=rt.block_on(async {
        let client=RobotClient::new(&config)?;
        Ok::<_,anyhow::Error>(JobExecutor::new(job().into(),Arc::new(config.clone()),sink.clone(),client).run().await)
    })?;
    anyhow::ensure!(outcome.urls.iter().any(|row|matches!(&row.outcome,FetchOutcome::DurableSuccess{final_url} if final_url.as_str()==destination.url.as_str())),"cross-site response was not captured: {outcome:?}");
    anyhow::ensure!(sink.0.lock().unwrap().len()==1,"redirect duplicated or lost a capture");
    let before=source.requests.lock().unwrap().len()+destination.requests.lock().unwrap().len();
    config.network_policy=CrawlNetworkPolicy::PublicOnly;
    let blocked=rt.block_on(async {
        let client=RobotClient::new(&config)?;
        Ok::<_,anyhow::Error>(JobExecutor::new(job().into(),Arc::new(config),sink.clone(),client).run().await)
    })?;
    anyhow::ensure!(!blocked.urls.iter().any(|row|matches!(row.outcome,FetchOutcome::DurableSuccess{..})),"public profile allowed loopback");
    let after=source.requests.lock().unwrap().len()+destination.requests.lock().unwrap().len();
    anyhow::ensure!(before==after,"public profile sent a private request");
    println!("{}",serde_json::json!({"passed":true,"cross_site_redirect_captured":true,"captures":1,
        "loopback_http_requests":before,"public_profile_private_requests":0,"outside_http_requests":0}));
    Ok(())
}
