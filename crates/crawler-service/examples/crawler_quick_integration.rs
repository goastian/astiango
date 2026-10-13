//! Short native integration scenarios. No requests leave the local fixture.
use anyhow::Result;
use astiango_crawler_service as crawler;
use std::{io::Write, path::Path, sync::Arc};

fn datum() -> crawler::crawler::CrawlDatum {
    let mut seed = 1234567u64;
    let body: String = (0..1048576).map(|_| {seed ^= seed << 13; seed ^= seed >> 7; seed ^= seed << 17; (b' '+(seed % 95) as u8) as char}).collect();
    crawler::crawler::CrawlDatum {capture:None,url:url::Url::parse("https://fixture.test/persisted").unwrap(),payload_type:crawler::warc::PayloadType::Html,body,fetch_time_ms:1,date:chrono::Utc::now()}
}
fn main() -> Result<()> {
    let args:Vec<_>=std::env::args().collect();
    anyhow::ensure!(args.len()==3,"usage: crawler_quick_integration memory|disk-quota|archive|recover|legacy-queue DIRECTORY");
    let directory=Path::new(&args[2]); std::fs::create_dir_all(directory)?;
    if args[1]=="legacy-queue" {
        use postcard::experimental::max_size::MaxSize;
        #[derive(serde::Serialize, MaxSize)] struct Header {body_size:usize}
        let mut data=std::fs::File::create(directory.join("data"))?;
        for i in 0..3 {
            let url=url::Url::parse(&format!("https://tenant{i}.blogspot.com/path/?sig=a%2Fb&x=1&x=2#fragment"))?;
            let job=crawler::crawler::Job {domain:crawler::crawler::Domain::from(&url), urls:std::collections::VecDeque::from([crawler::crawler::WeightedUrl {url,weight:1.0}]),wandering_urls:0};
            let bytes=bincode::encode_to_vec(job,bincode::config::standard())?;
            let mut header=postcard::to_allocvec(&Header {body_size:bytes.len()})?;
            header.resize(Header::POSTCARD_MAX_SIZE,0); data.write_all(&header)?; data.write_all(&bytes)?;
        }
        data.sync_all()?; std::fs::write(directory.join("pointer"),0u64.to_le_bytes())?;
        return Ok(());
    }
    if args[1]=="memory" {
        let config:crawler::crawler::local::Config=toml::from_str(&std::fs::read_to_string("configs/crawler/small.toml")?)?;
        config.validate()?;
        let runtime=tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        return runtime.block_on(async {
            let pipeline=crawler::crawler::pipeline::Pipeline::new(config.pipeline.clone())?;
            anyhow::ensure!(pipeline.stats().parse.workers==2,"fixture requires two effective CPU workers");
            let body:Arc<str>=format!("<html><head><meta name='robots' content='noindex'></head><body>{}</body></html>","<a href='/a'>x</a>".repeat(233000)).into();
            anyhow::ensure!(body.len()<=config.body.html.decoded_limit_bytes,"fixture exceeds body quota");
            let barrier=Arc::new(std::sync::Barrier::new(2));
            let mut tasks=Vec::new();
            for _ in 0..2 {
                let pipeline=pipeline.clone(); let body=body.clone(); let barrier=barrier.clone();
                tasks.push(tokio::spawn(async move {
                    let _admission=pipeline.admit(body.len()).await?;
                    pipeline.parse.run(body.len(),move || -> Result<_> {
                        let html=crawler::webpage::Html::parse_without_text(&body,"https://fixture.test/")?;
                        barrier.wait();
                        let robots=html.robots_for_agent(crawler::capture::DEFAULT_AGENT);
                        let anchors=html.visit_anchor_destinations(|_|true);
                        anyhow::ensure!(robots.no_index && anchors==233000,"adversarial document changed extraction semantics");
                        Ok(anchors)
                    }).await?
                }));
            }
            for task in tasks {task.await??;}
            pipeline.shutdown().await;
            let mut usage=std::mem::MaybeUninit::<libc::rusage>::uninit();
            anyhow::ensure!(unsafe {libc::getrusage(libc::RUSAGE_SELF,usage.as_mut_ptr())}==0,"getrusage failed");
            let peak=unsafe {usage.assume_init()}.ru_maxrss as u64*1024;
            anyhow::ensure!(peak<config.resources.memory_limit_bytes,"simultaneous DOMs exceeded 512 MiB profile: {peak}");
            println!("{}",serde_json::json!({"passed":true,"simultaneous_documents":2,"body_bytes_each":body.len(),"anchors_each":233000,"peak_rss_bytes":peak,"memory_limit_bytes":config.resources.memory_limit_bytes,"runtime_lag_p99_us":pipeline.stats().runtime_lag_p99_us,"cgroup_enforced":false}));
            Ok(())
        });
    }
    use crawler::crawler::spool::{Spool, SpoolConfig, SpoolMetrics};
    let metrics=SpoolMetrics::default();
    let config=SpoolConfig {directory:directory.to_path_buf(),..Default::default()};
    let mut spool=Spool::open(config.clone(),None,metrics.clone())?;
    if args[1]=="disk-full" {
        let mut first=datum(); first.body="previous durable capture".to_owned();
        spool.append(&[&first])?;
        let failed=spool.append(&[&datum()]);
        anyhow::ensure!(failed.is_err() && metrics.stats().records_durable==1,"ENOSPC published or removed an ACK");
        let reason=failed.unwrap_err().to_string();
        anyhow::ensure!(reason.contains("No space left"),"expected kernel ENOSPC, got {reason}");
        drop(spool);
        let recovered=SpoolMetrics::default();
        let _spool=Spool::open(config,None,recovered.clone())?;
        anyhow::ensure!(recovered.stats().records_durable==1,"ENOSPC recovery lost the previous ACK");
        println!("{}",serde_json::json!({"passed":true,"kernel_enospc":true,"prior_confirmed_retained":1,"new_false_ack":false,"error":reason}));
    } else if args[1]=="disk-quota" {
        let mut previous=std::mem::MaybeUninit::<libc::rlimit>::uninit();
        anyhow::ensure!(unsafe {libc::getrlimit(libc::RLIMIT_FSIZE,previous.as_mut_ptr())}==0,"getrlimit failed");
        let previous=unsafe {previous.assume_init()};
        let limited=libc::rlimit {rlim_cur:65536,rlim_max:previous.rlim_max};
        unsafe {libc::signal(libc::SIGXFSZ,libc::SIG_IGN);}
        anyhow::ensure!(unsafe {libc::setrlimit(libc::RLIMIT_FSIZE,&limited)}==0,"file quota unavailable");
        let failed=spool.append(&[&datum()]);
        anyhow::ensure!(unsafe {libc::setrlimit(libc::RLIMIT_FSIZE,&previous)}==0,"cannot restore quota");
        anyhow::ensure!(failed.is_err() && metrics.stats().records_durable==0,"failed append published an ACK");
        drop(spool);
        let _recovered=Spool::open(config,None,metrics.clone())?;
        anyhow::ensure!(metrics.stats().records_durable==0,"partial failed append recovered as confirmed");
        println!("{}",serde_json::json!({"passed":true,"kernel_file_size_quota_bytes":65536,"confirmed_after_failure":0,"error":failed.unwrap_err().to_string()}));
    } else if args[1]=="archive" {
        let receipt=spool.append(&[&datum()])?;
        std::fs::write(directory.join("ack.json"),serde_json::to_vec(&receipt)?)?;
        println!("ACK durable"); std::io::stdout().flush()?;
        loop {std::thread::park();}
    } else {
        anyhow::ensure!(args[1]=="recover","unknown scenario");
        anyhow::ensure!(metrics.stats().records_durable==1,"durable capture lost after process kill");
        println!("{}",serde_json::json!({"passed":true,"durable_after_kill":metrics.stats().records_durable}));
    }
    Ok(())
}
