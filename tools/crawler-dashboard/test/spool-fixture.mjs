export function spoolReport() {
  const metrics = Object.fromEntries(["pages_per_second", "cpu_ms_per_page", "compressed_bytes_per_page", "sync_calls_per_page", "files_per_page", "rss_peak_bytes"].map((key) => [key, { median: 10, min: 9, max: 11, mad: 1 }]));
  const identity = { scenario: "corpus_group_16", mode: "segments", gzip_level: 3, segment_bytes: 134217728 };
  return {
    schema_version: 1, kind: "crawler_spool_benchmark", generated_at: "2026-10-12T12:00:00Z",
    configuration: { acceptance_profile: "c08", smoke: true }, acceptance: { passed: true, full_profile: false },
    summaries: [{ ...identity, repetitions: 3, metrics }],
    runs: [1, 2, 3].map((repetition) => ({ ...identity, repetition, pages: 8, archive_verified: true,
      ...Object.fromEntries(Object.keys(metrics).map((key) => [key, 10])), sync_calls: 1, files: 1 })),
    comparisons: [{ scenario: identity.scenario, gzip_level: 3, equivalent: true,
      throughput_gain_pct: { median: -5, min: -6, max: -4, mad: 1 },
      sync_reduction_pct: { median: 90, min: 89, max: 91, mad: 1 },
      files_reduction_pct: { median: 90, min: 89, max: 91, mad: 1 } }],
    checks: [1, 2, 3].map((repetition) => ({ name: "crash_append_after", repetition, passed: true })),
  };
}
