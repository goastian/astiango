export function pipelineReport() {
  const metric = { median: 2, min: 1, max: 3, mad: 1 };
  const stage = { items: 0, bytes: 0, running: 0, queued: 0, peak_items: 2, peak_bytes: 100,
    peak_running: 1, max_items: 4, max_bytes: 1024, workers: 1, admission_waits: 2 };
  return { schema_version: 1, kind: "crawler_pipeline_benchmark", generated_at: "2026-10-12T12:00:00Z",
    configuration: { acceptance_profile: "c07" }, acceptance: { passed: true },
    summaries: ["legacy", "bounded"].map((mode) => ({ scenario: "slow_storage", mode, origins: 32,
      repetitions: 3, metrics: Object.fromEntries(["pages_per_second", "runtime_lag_p99_ms", "rss_peak_bytes",
        "cpu_ms_per_page", "controller_delay_ms", "admission_waits"].map((key) => [key, { ...metric }])) })),
    comparisons: [{ scenario: "slow_storage", equivalent: true, throughput_gain_pct: { ...metric }, lag_reduction_pct: { ...metric } }],
    runs: ["legacy", "bounded"].flatMap((mode) => [1, 2, 3].map((repetition) => ({ scenario: "slow_storage", mode,
      repetition, pages: 24, pages_per_second: 2, runtime_lag_p99_ms: 2, rss_peak_bytes: 100, controller_delay_ms: 2,
      server: { requests: 24 }, pipeline: Object.fromEntries(["admission", "parse", "network_setup", "storage", "control", "batch_queue"].map((key) => [key, { ...stage }])) }))) };
}
