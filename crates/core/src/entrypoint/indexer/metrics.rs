use serde::Serialize;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    time::Instant,
};
pub static ENABLED: AtomicBool = AtomicBool::new(false);
pub static ALLOCATED: AtomicU64 = AtomicU64::new(0);
static WALL: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static CPU: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
static BYTES: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];
pub const NAMES: [&str; 7] = [
    "read_decode",
    "dom",
    "text_extraction",
    "signals",
    "tokenization",
    "commit",
    "merge",
];
pub fn cpu_ns() -> u64 {
    let mut usage: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut usage) };
    ((usage.ru_utime.tv_sec + usage.ru_stime.tv_sec) as u64 * 1_000_000
        + (usage.ru_utime.tv_usec + usage.ru_stime.tv_usec) as u64)
        * 1000
}
pub struct Span(Option<(usize, Instant, u64, u64)>);
impl Span {
    pub fn new(stage: usize) -> Self {
        Self(ENABLED.load(Ordering::Relaxed).then(|| {
            (
                stage,
                Instant::now(),
                cpu_ns(),
                ALLOCATED.load(Ordering::Relaxed),
            )
        }))
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some((stage, started, cpu, bytes)) = self.0 {
            WALL[stage].fetch_add(started.elapsed().as_nanos() as u64, Ordering::Relaxed);
            CPU[stage].fetch_add(cpu_ns().saturating_sub(cpu), Ordering::Relaxed);
            BYTES[stage].fetch_add(
                ALLOCATED.load(Ordering::Relaxed).saturating_sub(bytes),
                Ordering::Relaxed,
            );
        }
    }
}
#[derive(Serialize)]
pub struct Measurement {
    pub wall_ms: f64,
    pub cpu_ms: f64,
    pub allocated_bytes: u64,
}
pub fn snapshot() -> BTreeMap<String, Measurement> {
    NAMES
        .iter()
        .enumerate()
        .map(|(i, name)| {
            (
                name.to_string(),
                Measurement {
                    wall_ms: WALL[i].load(Ordering::Relaxed) as f64 / 1e6,
                    cpu_ms: CPU[i].load(Ordering::Relaxed) as f64 / 1e6,
                    allocated_bytes: BYTES[i].load(Ordering::Relaxed),
                },
            )
        })
        .collect()
}
