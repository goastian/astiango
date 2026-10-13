use anyhow::Result;
use astiango::crawler::{
    runtime::{IoFuture, SystemClock},
    transport::{DnsAnswer, FixturePolicy, SharedTransport, TransportResolver},
};
use astiango_crawler_service as astiango;
use std::{
    collections::BTreeSet,
    net::{IpAddr, Ipv4Addr},
    sync::Arc,
};

struct Resolver(BTreeSet<String>);
impl TransportResolver for Resolver {
    fn resolve<'a>(
        &'a self,
        host: &'a str,
    ) -> IoFuture<'a, std::result::Result<DnsAnswer, astiango::crawler::Error>> {
        Box::pin(async move {
            if !self.0.contains(host) {
                return Err(anyhow::anyhow!("fixture DNS destination not admitted").into());
            }
            Ok(DnsAnswer {
                addresses: vec![IpAddr::V4(Ipv4Addr::LOCALHOST)],
                ttl: std::time::Duration::from_secs(60),
            })
        })
    }
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() == 3 && args[1] == "resources" {
        use astiango::crawler::resources::{Config, Controller};
        let directory = std::path::PathBuf::from(&args[2]);
        std::fs::create_dir_all(&directory)?;
        let config = Config {
            automatic: true,
            memory_limit_bytes: 512 * 1024 * 1024,
            max_workers: 20,
            ..Default::default()
        };
        let controller = Controller::new(config.clone(), directory.clone(), 20)?;
        let maximum = controller.sample(0, 0, 8 * 1024 * 1024 * 1024)?;
        anyhow::ensure!(
            controller.stats().effective_cpus == 2 && maximum == 2,
            "real cgroup must cap twenty requested workers at two"
        );
        anyhow::ensure!(
            controller.sample(0, 8 * 1024 * 1024 * 1024, 8 * 1024 * 1024 * 1024)? == 0,
            "spool pressure did not stop admission"
        );
        let spool = astiango::crawler::spool::SpoolStats {
            max_segments: 256,
            segments_total: 255,
            max_spool_bytes: 8 * 1024 * 1024 * 1024,
            ..Default::default()
        };
        anyhow::ensure!(
            controller.sample_spool(0, &spool)? == 0,
            "segment quota did not reserve space for inflight work"
        );
        let mut recovery = Vec::new();
        for _ in 0..6 {
            recovery.push(controller.sample(0, 0, 8 * 1024 * 1024 * 1024)?);
        }
        anyhow::ensure!(
            recovery == [0, 0, 1, 1, 1, 2],
            "admission hysteresis changed: {:?}",
            recovery
        );
        let disk = Controller::new(
            Config {
                minimum_disk_free_bytes: u64::MAX,
                ..config.clone()
            },
            directory.join("disk"),
            20,
        )?;
        disk.sample(0, 0, 8 * 1024 * 1024 * 1024)?;
        anyhow::ensure!(
            disk.stats().reason == "disk_reserve" && disk.stats().workers_allowed == 0,
            "disk reserve was ignored"
        );
        let inode = Controller::new(
            Config {
                minimum_free_inodes: u64::MAX,
                ..config.clone()
            },
            directory.join("inode"),
            20,
        )?;
        inode.sample(0, 0, 8 * 1024 * 1024 * 1024)?;
        anyhow::ensure!(
            inode.stats().reason == "inode_reserve" && inode.stats().workers_allowed == 0,
            "inode reserve was ignored"
        );
        let fd = Controller::new(
            Config {
                max_open_fds: 32,
                ..config.clone()
            },
            directory.join("fd"),
            20,
        )?;
        let held = (0..40)
            .map(|_| std::fs::File::open("/dev/null"))
            .collect::<std::io::Result<Vec<_>>>()?;
        fd.sample(0, 0, 8 * 1024 * 1024 * 1024)?;
        anyhow::ensure!(
            fd.stats().reason == "fd_limit" && fd.stats().workers_allowed == 0,
            "FD pressure was ignored"
        );
        drop(held);
        let quota_config = Config {
            daily_network_bytes: 1024,
            ..config.clone()
        };
        let quota = Controller::new(quota_config.clone(), directory.join("quota"), 20)?;
        anyhow::ensure!(
            quota.sample(2048, 0, 8 * 1024 * 1024 * 1024)? == 0,
            "network quota was ignored"
        );
        drop(quota);
        let quota = Controller::new(quota_config, directory.join("quota"), 20)?;
        anyhow::ensure!(
            quota.sample(0, 0, 8 * 1024 * 1024 * 1024)? == 0
                && quota.stats().network_bytes_today == 2048,
            "network quota was lost on restart"
        );
        let memory = Controller::new(
            Config {
                memory_limit_bytes: 128 * 1024 * 1024,
                ..config
            },
            directory.join("memory"),
            20,
        )?;
        let pressure = vec![42u8; 144 * 1024 * 1024];
        std::hint::black_box(&pressure);
        memory.sample(0, 0, 8 * 1024 * 1024 * 1024)?;
        anyhow::ensure!(
            memory.stats().reason == "memory_limit" && memory.stats().workers_allowed == 0,
            "RSS pressure was ignored"
        );
        let peak_rss = memory.stats().rss_bytes;
        drop(pressure);
        memory.sample(0, 0, 8 * 1024 * 1024 * 1024)?;
        let baseline = memory.stats().rss_bytes as usize;
        let pressure = vec![42u8; (120usize * 1024 * 1024).saturating_sub(baseline)];
        std::hint::black_box(&pressure);
        memory.sample(0, 0, 8 * 1024 * 1024 * 1024)?;
        anyhow::ensure!(
            memory.stats().reason == "memory_pressure" && memory.stats().workers_allowed == 0,
            "memory pressure resumed admission before healthy hysteresis: reason={}, rss={}",
            memory.stats().reason,
            memory.stats().rss_bytes
        );
        anyhow::ensure!(peak_rss < 512 * 1024 * 1024, "profile exceeded 512 MiB");
        println!(
            "{}",
            serde_json::json!({"passed":true,"effective_cpus":2,"memory_max_bytes":536870912,"peak_rss_bytes":peak_rss,"recovery":recovery,"disk_reserve":true,"segment_quota":true,"inode_reserve":true,"fd_limit":true,"durable_network_quota":true,"memory_pressure":true,"memory_pressure_hysteresis":true})
        );
        return Ok(());
    }
    if args.len() == 3 && args[1] == "restore" {
        let directory = std::path::PathBuf::from(&args[2]);
        let frontier = astiango::crawler::frontier::Frontier::open_database(
            directory.join("frontier.json"),
            10000,
            50,
        )?;
        frontier.reconcile_spool(&directory.join("warc/spool"))?;
        let stats = frontier.metrics.snapshot();
        println!(
            "{}",
            serde_json::json!({"passed":true,"frontier_commits":stats.commits,"urls":stats.urls,"outbox_entries":stats.outbox_entries,"reconciled_captures":stats.reconciled_captures})
        );
        return Ok(());
    }
    anyhow::ensure!(
        args.len() == 3,
        "usage: crawler_node_lab WORKER_TOML HOSTS_JSON"
    );
    let config: astiango::crawler::leased::Config =
        toml::from_str(&std::fs::read_to_string(&args[1])?)?;
    let hosts: BTreeSet<String> = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    let options = config.crawler.crawler_config();
    let transport = SharedTransport::with_fixture_resolver(
        &options,
        options.transport.clone(),
        Arc::new(Resolver(hosts.clone())),
        Arc::new(SystemClock),
        FixturePolicy {
            allowed_hosts: hosts,
            certificate_pem: None,
        },
    )?;
    let cpus = astiango::crawler::resources::effective_cpus();
    eprintln!("fixture runtime effective_cpus={cpus}");
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(cpus.min(4))
        .enable_all()
        .build()?
        .block_on(astiango::crawler::leased::run_fixture(config, transport))
}
