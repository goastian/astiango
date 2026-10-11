export type Distribution = {
  median: number;
  min: number;
  max: number;
  mad: number;
};
export type BenchmarkRun = {
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
  }[];
  acceptance: { passed: boolean; [key: string]: unknown };
};
