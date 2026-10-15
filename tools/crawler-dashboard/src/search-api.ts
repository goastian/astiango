export type SearchResponse = {
  snapshot: {
    schema_version: number; component: string; run_id: string; pid: number;
    started_at: string; updated_at: string; phase: string; reason: string; ready: boolean;
    generation: null | { id: string; documents: number; manifest_sha256: string; signals?: unknown };
    previous: string | null; desired: string | null;
    totals: Record<string, number>;
    window: { qps: number; p50_ms: number; p95_ms: number; p99_ms: number; samples: number };
    active: number; queued: number;
    resources: { rss_bytes: number; cpu_percent: number; open_fds: number; read_bytes?: number; written_bytes?: number };
    cache_bytes?: number;
    nodes: { id: string; pid: number; generation: string; state: string; load_ms: number; requests: number }[];
    history: { at: string; qps: number; p95_ms: number | null; rss_bytes: number }[];
    stages_ms: Record<string, number>;
    configuration: Record<string, unknown>; capabilities: Record<string, unknown>;
    generations: { id: string; state: string; documents: number; selected: boolean; created_at?: string }[];
    commands: { id: string; action: string; state: string; generation?: string; error?: string }[];
    events: { at: string; reason: string; detail: string }[];
    last_probe?: { at: string; state: string; generation?: string; results?: number; wall_ms?: number } | null;
    last_user_query_at?: string | null;
  };
  meta: { read_at: string; process_alive: boolean };
};
