export function transportReport() {
  const metric = (value) => ({ median: value, mad: 0, min: value, max: value });
  const summaries = ["http", "https"].flatMap((scheme) => ["legacy", "pooled"].map((mode) => {
    const connections = mode === "pooled" ? 2 : 24;
    return {
      scheme, origins: 2, mode, repetitions: 3,
      metrics: {
        duration_ms: metric(120), requests: metric(24), pages: metric(18),
        connections: metric(connections), connections_per_page: metric(connections / 18),
        dns_queries: metric(connections), dns_queries_per_page: metric(connections / 18),
        fd_baseline: metric(5), fd_peak: metric(12), fd_final: metric(5),
      },
    };
  }));
  return {
    schema_version: 1, kind: "crawler_transport_benchmark", generated_at: new Date().toISOString(),
    configuration: { acceptance_profile: "c04", origins: [2], repetitions: 3 },
    summaries,
    runs: summaries.flatMap((row) => [1, 2, 3].map((repetition) => ({
      scheme: row.scheme, origins: row.origins, mode: row.mode, repetition,
      requests: 24, pages: 18, fd_baseline: 5, fd_peak: 12, fd_final: 5,
      server: { requests: 24, connections: row.metrics.connections.median, peak_connections: 2 },
      transport_stats: { clients_created: 999, clients_reused: 999 },
      verification: { passed: true },
    }))),
    comparisons: ["http", "https"].map((scheme) => ({
      scheme, origins: 2, equivalent: true,
      connections_per_page_reduction_pct: metric(100 * (1 - 2 / 24)),
      dns_queries_per_page_reduction_pct: metric(100 * (1 - 2 / 24)),
    })),
    acceptance: { passed: true, checks: { all_runs_verified: true } },
    code: { commit: "test-c04", source_sha256: "test-source" },
    hardware: { cpu_count: 8 }, corpus: { sha256: "test-corpus" }, source_unchanged: true,
  };
}
