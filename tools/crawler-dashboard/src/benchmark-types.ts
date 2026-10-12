export type Distribution = {
  median: number;
  min: number;
  max: number;
  mad: number;
};
export type BenchmarkRun = {
  observer_mode?: "legacy_sync" | "async" | "off";
  repetition?: number;
  errors?: Record<string, number>;
  request_failures?: number;
  saved_pages?: number;
  client_requests?: number;
  requests_match?: boolean;
  server?: { requests?: number; statuses?: Record<string, number> };
};
export type BenchmarkScenario = {
  name: string;
  origins: number;
  observer: boolean | "on" | "off" | "enabled" | "disabled";
  observer_mode?: "legacy_sync" | "async" | "off";
  repetitions: number;
  requests_match: boolean;
  runs?: BenchmarkRun[];
  metrics: {
    duration_ms: Distribution;
    cpu_seconds?: Distribution;
    rss_peak_bytes?: Distribution;
    requests?: Distribution;
    saved_pages?: Distribution;
    stages_ms?: Record<string, Distribution>;
    observer_stats?: Record<string, Distribution>;
    [name: string]: Distribution | Record<string, Distribution> | undefined;
  };
};
export type BenchmarkReport = {
  schema_version: 1;
  kind: "crawler-capacity-benchmark";
  generated_at: string;
  code?: { commit?: string; dirty?: boolean; source_sha256?: unknown };
  corpus?: { sha256?: string; files?: unknown[] };
  hardware?: Record<string, unknown>;
  configuration: Record<string, unknown>;
  scenarios: BenchmarkScenario[];
  comparisons?: {
    name: string;
    origins: number;
    duration_overhead_pct?: number | null;
    requests_equal: boolean;
    saved_pages_equal: boolean;
    behavior_equivalent?: boolean;
    write_calls_reduction_pct?: Distribution;
    writes_per_update_reduction_pct?: Distribution;
  }[];
  acceptance: { passed: boolean; [key: string]: unknown };
};

export type TransportBenchmarkSummary = {
  scheme: "http" | "https";
  origins: number;
  mode: "legacy" | "pooled";
  repetitions: number;
  metrics: Record<string, Distribution>;
};
export type TransportBenchmarkRun = {
  scheme: "http" | "https";
  origins: number;
  mode: "legacy" | "pooled";
  repetition: number;
  pages: number;
  requests: number;
  fd_baseline?: number;
  fd_peak?: number;
  fd_final?: number;
  server: { connections: number; requests: number; peak_connections?: number };
  verification?: { passed: boolean };
};
export type TransportBenchmarkReport = {
  schema_version: 1;
  kind: "crawler_transport_benchmark";
  generated_at: string;
  configuration: Record<string, unknown>;
  summaries: TransportBenchmarkSummary[];
  runs: TransportBenchmarkRun[];
  comparisons?: {
    scheme: "http" | "https";
    origins: number;
    equivalent: boolean;
    connections_per_page_reduction_pct: Distribution;
    dns_queries_per_page_reduction_pct?: Distribution;
  }[];
  acceptance: { passed: boolean; checks?: Record<string, boolean> };
  code?: Record<string, unknown>;
  corpus?: Record<string, unknown>;
  hardware?: Record<string, unknown>;
  source_unchanged?: boolean;
};
