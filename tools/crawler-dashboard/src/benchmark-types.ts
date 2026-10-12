export type Distribution = {
  median: number;
  min: number;
  max: number;
  mad: number;
};
export type PipelineBenchmarkReport = {
  schema_version: 1;
  kind: "crawler_pipeline_benchmark";
  generated_at: string;
  configuration: { acceptance_profile: "c07"; [key: string]: unknown };
  summaries: { scenario: string; mode: "legacy" | "bounded"; origins: number; repetitions: number; metrics: Record<string, Distribution> }[];
  comparisons: { scenario: string; equivalent: boolean; throughput_gain_pct: Distribution; lag_reduction_pct: Distribution }[];
  runs: { scenario: string; mode: "legacy" | "bounded"; repetition: number; pages: number; pages_per_second: number;
    runtime_lag_p99_ms: number; controller_delay_ms: number; rss_peak_bytes: number;
    pipeline: import("./types").PipelineStats; server: { requests: number; body_bytes_written: number } }[];
  acceptance: { passed: boolean; checks?: Record<string, boolean> };
  code?: Record<string, unknown>;
  binary?: Record<string, unknown>;
  hardware?: Record<string, unknown>;
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

export type BodyBenchmarkSummary = {
  scenario: string;
  expected_outcome: string;
  passed: boolean;
  repetitions: number;
  metrics: Record<string, Distribution>;
};
export type BodyBenchmarkRun = {
  scenario: string;
  repetition: number;
  configuration?: Record<string, unknown>;
  outcomes: { path: string; cause: string; decoded_bytes: number }[];
  body_stats: { wire_bytes_received: number; decoded_bytes_produced: number; [key: string]: unknown };
  server: { requests: number; body_bytes_planned: number; body_bytes_written: number };
  verification: { passed: boolean; checks?: Record<string, boolean> };
};
export type BodyBenchmarkReport = {
  schema_version: 1;
  kind: "crawler_body_benchmark";
  generated_at: string;
  configuration: { acceptance_profile: "c05"; profile: Record<string, unknown>; [key: string]: unknown };
  summaries: BodyBenchmarkSummary[];
  runs: BodyBenchmarkRun[];
  acceptance: { passed: boolean; checks?: Record<string, boolean> };
  code?: Record<string, unknown>;
  binary?: Record<string, unknown>;
  build?: Record<string, unknown>;
  hardware?: Record<string, unknown>;
};

export type ExtractionBenchmarkReport = {
  schema_version: 1;
  kind: "crawler_extraction_benchmark";
  generated_at: string;
  configuration: { acceptance_profile: "c06"; [key: string]: unknown };
  summaries: { scenario: string; mode: "legacy" | "shared"; repetitions: number; metrics: Record<string, Distribution> }[];
  comparisons: { scenario: string; equivalent: boolean; cpu_reduction_pct: Distribution; allocated_bytes_reduction_pct: Distribution }[];
  corpus_cpu_reduction_pct: Distribution;
  runs: { scenario: string; mode: "legacy" | "shared"; repetition: number; cpu_ms_per_page: number;
    allocated_bytes_per_page: number; allocation_calls_per_page: number; dom_parses_per_page: number;
    extraction: { anchors_visited: number; discovery_urls: string[]; internal_urls: string[]; base_url: string;
      no_index: boolean; no_follow: boolean; traversal_stopped: boolean } }[];
  acceptance: { passed: boolean; checks?: Record<string, boolean> };
  code?: Record<string, unknown>;
  binary?: Record<string, unknown>;
  hardware?: Record<string, unknown>;
};
export interface SpoolBenchmarkReport {
  schema_version: 1; kind: "crawler_spool_benchmark"; generated_at: string;
  configuration: { acceptance_profile: "c08"; smoke: boolean };
  acceptance: { passed: boolean; full_profile: boolean };
  summaries: Array<{ scenario: string; mode: "legacy_page" | "segments"; gzip_level: number;
    segment_bytes: number; repetitions: number; metrics: Record<string, Distribution> }>;
  comparisons: Array<{ scenario: string; gzip_level: number; equivalent: boolean;
    throughput_gain_pct: Distribution; sync_reduction_pct: Distribution; files_reduction_pct: Distribution }>;
  runs: Array<{ scenario: string; mode: "legacy_page" | "segments"; gzip_level: number; segment_bytes: number;
    repetition: number; pages: number; pages_per_second: number; cpu_ms_per_page: number;
    compressed_bytes_per_page: number; sync_calls: number; files: number; rss_peak_bytes: number }>;
  checks: Array<{ name: string; repetition: number; passed: boolean }>;
}
