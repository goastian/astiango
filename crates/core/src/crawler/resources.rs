use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub automatic: bool,
    pub memory_limit_bytes: u64,
    pub minimum_disk_free_bytes: u64,
    pub minimum_free_inodes: u64,
    pub spool_reserve_bytes: u64,
    pub max_open_fds: usize,
    pub daily_network_bytes: u64,
    pub min_workers: usize,
    pub max_workers: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            automatic: false,
            memory_limit_bytes: 1024 * 1024 * 1024,
            minimum_disk_free_bytes: 256 * 1024 * 1024,
            minimum_free_inodes: 1024,
            spool_reserve_bytes: 128 * 1024 * 1024,
            max_open_fds: 1024,
            daily_network_bytes: u64::MAX,
            min_workers: 1,
            max_workers: 2,
        }
    }
}

impl Config {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.memory_limit_bytes >= 128 * 1024 * 1024 && self.minimum_disk_free_bytes > 0,
            "resource memory must be at least 128 MiB and disk reserve positive"
        );
        anyhow::ensure!(
            self.min_workers > 0
                && self.min_workers <= self.max_workers
                && self.max_workers <= 32
                && self.max_open_fds >= 32
                && self.daily_network_bytes > 0
                && self.spool_reserve_bytes > 0,
            "invalid resource admission limits"
        );
        Ok(())
    }
}

#[derive(Clone, Default, Serialize)]
pub struct Stats {
    pub automatic: bool,
    pub effective_cpus: usize,
    pub rss_bytes: u64,
    pub process_cpu_percent: f64,
    pub process_read_bytes: u64,
    pub process_written_bytes: u64,
    pub memory_current_bytes: Option<u64>,
    pub memory_limit_bytes: u64,
    pub disk_free_bytes: u64,
    pub free_inodes: u64,
    pub open_fds: usize,
    pub workers_allowed: usize,
    pub workers_max: usize,
    pub reason: String,
    pub adjustments: u64,
    pub network_bytes_today: u64,
    pub daily_network_bytes: u64,
    pub sampled_at: String,
}

#[derive(Clone)]
pub struct Controller {
    config: Config,
    directory: PathBuf,
    stats: Arc<Mutex<Stats>>,
    healthy: Arc<Mutex<u8>>,
    cpu_sample: Arc<Mutex<Option<(u64, std::time::Instant)>>>,
    quota: Arc<Mutex<Quota>>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Quota {
    version: u32,
    date: String,
    received_bytes: u64,
    #[serde(skip)]
    last_received: u64,
}

impl Controller {
    pub fn new(config: Config, directory: PathBuf, requested: usize) -> anyhow::Result<Self> {
        config.validate()?;
        let max = if config.automatic {
            requested.min(config.max_workers)
        } else {
            requested
        }
        .min(effective_cpus())
        .max(1);
        let stats = Stats {
            automatic: config.automatic,
            effective_cpus: effective_cpus(),
            workers_allowed: max,
            workers_max: max,
            memory_limit_bytes: config.memory_limit_bytes,
            daily_network_bytes: config.daily_network_bytes,
            reason: "manual".into(),
            ..Default::default()
        };
        let quota_path = directory.join("network-quota.json");
        let quota = if config.daily_network_bytes != u64::MAX && quota_path.exists() {
            let quota: Quota =
                serde_json::from_reader(std::fs::File::open(quota_path)?.take(65536))?;
            anyhow::ensure!(quota.version == 1, "unsupported network quota version");
            quota
        } else {
            Quota {
                version: 1,
                ..Default::default()
            }
        };
        Ok(Self {
            config,
            directory,
            stats: Arc::new(Mutex::new(stats)),
            healthy: Arc::new(Mutex::new(0)),
            cpu_sample: Arc::new(Mutex::new(None)),
            quota: Arc::new(Mutex::new(quota)),
        })
    }

    pub fn stats(&self) -> Stats {
        self.stats.lock().unwrap().clone()
    }

    pub fn sample_spool(
        &self,
        received: u64,
        spool: &super::spool::SpoolStats,
    ) -> anyhow::Result<usize> {
        let reserve = usize::from(spool.max_segments > 1);
        let segments_full = spool.max_segments > 0
            && spool.segments_total >= spool.max_segments.saturating_sub(reserve) as u64;
        let pressure_bytes = if segments_full {
            spool.max_spool_bytes
        } else {
            spool.spool_bytes
        };
        self.sample(received, pressure_bytes, spool.max_spool_bytes)
    }

    pub fn sample(
        &self,
        received: u64,
        spool_bytes: u64,
        spool_limit: u64,
    ) -> anyhow::Result<usize> {
        let status = fs::read_to_string("/proc/self/status")?;
        let rss = status
            .lines()
            .find_map(|line| {
                line.strip_prefix("VmRSS:")
                    .and_then(|v| v.split_whitespace().next()?.parse::<u64>().ok())
            })
            .unwrap_or(0)
            * 1024;
        let cg = cgroup_directory();
        let memory_current = cg
            .as_ref()
            .and_then(|path| number(path.join("memory.current")));
        let memory_limits: Vec<_> = cg
            .as_ref()
            .into_iter()
            .flat_map(|path| cgroup_ancestors(path))
            .filter_map(|path| {
                Some((
                    number(path.join("memory.max"))?,
                    number(path.join("memory.current"))?,
                ))
            })
            .collect();
        let memory_max = memory_limits.iter().map(|(limit, _)| *limit).min();
        let memory_limit = memory_max
            .unwrap_or(self.config.memory_limit_bytes)
            .min(self.config.memory_limit_bytes);
        let mut directory = self.directory.as_path();
        while !directory.exists() {
            directory = directory.parent().unwrap_or(Path::new("."));
        }
        let (free, inodes) = disk_available(directory)?;
        let fds = fs::read_dir("/proc/self/fd")?.count();
        let mut stats = self.stats.lock().unwrap();
        let prior = stats.workers_allowed;
        let now = chrono::Utc::now().to_rfc3339();
        let mut quota = self.quota.lock().unwrap();
        if quota.date != now[..10] {
            quota.received_bytes = 0;
            quota.date = now[..10].into();
        }
        quota.received_bytes = quota
            .received_bytes
            .saturating_add(received.saturating_sub(quota.last_received));
        quota.last_received = received;
        if self.config.daily_network_bytes != u64::MAX {
            fs::create_dir_all(&self.directory)?;
            super::super::warc::consumer::atomic_json(
                &self.directory.join("network-quota.json"),
                &*quota,
            )?;
        }
        stats.network_bytes_today = quota.received_bytes;
        let received_today = quota.received_bytes;
        drop(quota);
        if let Ok(process) = fs::read_to_string("/proc/self/stat") {
            let fields = process
                .rsplit(')')
                .next()
                .unwrap_or("")
                .split_whitespace()
                .collect::<Vec<_>>();
            if let (Some(user), Some(system)) = (
                fields.get(11).and_then(|v| v.parse::<u64>().ok()),
                fields.get(12).and_then(|v| v.parse::<u64>().ok()),
            ) {
                let ticks = user.saturating_add(system);
                let now = std::time::Instant::now();
                let mut previous = self.cpu_sample.lock().unwrap();
                if let Some((prior_ticks, prior_time)) = *previous {
                    let seconds = now.duration_since(prior_time).as_secs_f64();
                    let frequency = unsafe { libc::sysconf(libc::_SC_CLK_TCK) }.max(1) as f64;
                    stats.process_cpu_percent = 100.0 * ticks.saturating_sub(prior_ticks) as f64
                        / frequency
                        / seconds.max(0.001);
                }
                *previous = Some((ticks, now));
            }
        }
        if let Ok(io) = fs::read_to_string("/proc/self/io") {
            for line in io.lines() {
                if let Some((key, value)) = line.split_once(':') {
                    let bytes = value.trim().parse().unwrap_or(0);
                    if key == "read_bytes" {
                        stats.process_read_bytes = bytes;
                    }
                    if key == "write_bytes" {
                        stats.process_written_bytes = bytes;
                    }
                }
            }
        }
        let used = rss;
        let cgroup_full = memory_limits
            .iter()
            .any(|(limit, current)| current >= limit);
        let cgroup_pressure = memory_limits
            .iter()
            .any(|(limit, current)| *current >= limit.saturating_mul(9) / 10);
        let reason = if spool_bytes.saturating_add(self.config.spool_reserve_bytes) >= spool_limit {
            "spool_full"
        } else if free < self.config.minimum_disk_free_bytes {
            "disk_reserve"
        } else if inodes < self.config.minimum_free_inodes {
            "inode_reserve"
        } else if fds >= self.config.max_open_fds {
            "fd_limit"
        } else if used >= memory_limit || cgroup_full {
            "memory_limit"
        } else if received_today >= self.config.daily_network_bytes {
            "network_budget"
        } else if used >= memory_limit.saturating_mul(9) / 10 || cgroup_pressure {
            "memory_pressure"
        } else {
            "healthy"
        };
        if self.config.automatic {
            if reason == "memory_pressure" {
                stats.workers_allowed = if prior == 0 {
                    0
                } else {
                    (prior / 2)
                        .max(self.config.min_workers)
                        .min(stats.workers_max)
                };
                *self.healthy.lock().unwrap() = 0;
            } else if reason != "healthy" {
                stats.workers_allowed = 0;
                *self.healthy.lock().unwrap() = 0;
            } else if used < memory_limit.saturating_mul(3) / 4
                && free >= self.config.minimum_disk_free_bytes.saturating_mul(2)
            {
                let mut healthy = self.healthy.lock().unwrap();
                *healthy = healthy.saturating_add(1);
                if *healthy >= 3 {
                    stats.workers_allowed = (prior + 1).min(stats.workers_max);
                    *healthy = 0;
                }
            }
        }
        if stats.workers_allowed != prior {
            stats.adjustments += 1;
        }
        stats.reason = if !self.config.automatic {
            "manual".into()
        } else if reason == "healthy" && stats.workers_allowed < stats.workers_max {
            "recovering".into()
        } else {
            reason.into()
        };
        stats.rss_bytes = rss;
        stats.memory_current_bytes = memory_current;
        stats.memory_limit_bytes = memory_limit;
        stats.disk_free_bytes = free;
        stats.free_inodes = inodes;
        stats.open_fds = fds;
        stats.sampled_at = now;
        Ok(stats.workers_allowed)
    }
}

fn number(path: impl AsRef<Path>) -> Option<u64> {
    fs::read_to_string(path).ok()?.trim().parse().ok()
}

fn cgroup_directory() -> Option<PathBuf> {
    let content = fs::read_to_string("/proc/self/cgroup").ok()?;
    let relative = content.lines().find_map(|line| line.strip_prefix("0::"))?;
    let root = Path::new("/sys/fs/cgroup");
    let path = root.join(relative.trim_start_matches('/'));
    if path.is_dir() {
        Some(path)
    } else if root.join("cgroup.controllers").exists() {
        Some(root.into())
    } else {
        None
    }
}

fn cgroup_ancestors(path: &Path) -> impl Iterator<Item = &Path> {
    path.ancestors()
        .take_while(|ancestor| ancestor.starts_with("/sys/fs/cgroup"))
}

pub fn effective_cpus() -> usize {
    let available = std::thread::available_parallelism().map_or(1, usize::from);
    let quota = cgroup_directory().and_then(|directory| {
        cgroup_ancestors(&directory)
            .filter_map(|path| {
                fs::read_to_string(path.join("cpu.max"))
                    .ok()
                    .and_then(|text| cpu_quota(&text))
            })
            .min()
    });
    available.min(quota.unwrap_or(available)).max(1)
}

pub fn cpu_quota(text: &str) -> Option<usize> {
    let mut values = text.split_whitespace();
    let quota: u64 = values.next()?.parse().ok()?;
    let period: u64 = values.next()?.parse().ok()?;
    (period > 0).then(|| quota.div_ceil(period).max(1) as usize)
}

fn disk_available(path: &Path) -> anyhow::Result<(u64, u64)> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    let result = unsafe { libc::statvfs(path.as_ptr(), stats.as_mut_ptr()) };
    if result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let stats = unsafe { stats.assume_init() };
    Ok((
        stats.f_bavail.saturating_mul(stats.f_frsize),
        stats.f_favail,
    ))
}
