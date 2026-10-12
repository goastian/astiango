export function extractionStats() {
  return { documents: 10, dom_parses: 10, failures: 0, anchors_visited: 1000,
    discovery_links: 700, internal_links: 400, pages_at_discovery_limit: 1,
    pages_at_internal_limit: 1, traversals_stopped: 1, wall_us: 25000,
    cpu_us: 20000, cpu_samples: 10, max_links_per_projection: 512 };
}
export function extractionReport() {
  const metric = (value) => ({ median: value, mad: 0, min: value, max: value });
  const modes = ["legacy", "shared"];
  const values = (mode) => ({ cpu_ms_per_page: mode === "legacy" ? 10 : 4,
    wall_ms_per_page: mode === "legacy" ? 11 : 5, allocated_bytes_per_page: mode === "legacy" ? 1000 : 400,
    allocation_calls_per_page: mode === "legacy" ? 100 : 40, dom_parses_per_page: mode === "legacy" ? 2 : 1 });
  return { schema_version: 1, kind: "crawler_extraction_benchmark", generated_at: new Date().toISOString(),
    configuration: { acceptance_profile: "c06", network: "none" }, corpus_cpu_reduction_pct: metric(60),
    summaries: modes.map((mode) => ({ scenario: "article", mode, repetitions: 3,
      metrics: Object.fromEntries(Object.entries(values(mode)).map(([key, value]) => [key, metric(value)])) })),
    comparisons: [{ scenario: "article", equivalent: true, cpu_reduction_pct: metric(60), allocated_bytes_reduction_pct: metric(60) }],
    runs: [1, 2, 3].flatMap((repetition) => modes.map((mode) => ({ scenario: "article", mode, repetition, ...values(mode),
      extraction: { anchors_visited: 1, base_url: "https://example.org/", no_index: false, no_follow: false,
        traversal_stopped: false, discovery_urls: ["https://other.org/"], internal_urls: [] } }))),
    acceptance: { passed: true, checks: { equivalent_links_metadata: true } } };
}
