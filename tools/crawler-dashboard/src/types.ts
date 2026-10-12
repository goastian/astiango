export type SiteStatus =
  | "queued"
  | "crawling"
  | "waiting"
  | "completed"
  | "stopped"
  | "deferred"
  | "cancelled"
  | "storage_failed"
  | "failed";
export interface ExplicitCounters {
  page_http_attempts?: number;
  redirect_hops?: number;
  useful_pages?: number;
  deferred?: number;
  cancelled?: number;
  storage_failed?: number;
}
export interface Site extends ExplicitCounters {
  host: string;
  status: SiteStatus;
  current_url: string | null;
  pages_downloaded: number;
  failed: number;
  skipped: number;
  attempted: number;
  budget: number;
  bytes: number;
  last_activity_at: string | null;
  last_http_status: number | null;
  last_error: string | null;
  last_fetch_ms: number | null;
  next_request_at: string | null;
}
export interface CrawlEvent {
  at: string;
  level: "info" | "warn" | "error";
  kind: string;
  message: string;
  host: string | null;
  url: string | null;
  http_status: number | null;
  bytes: number | null;
  fetch_time_ms: number | null;
  warc_file: string | null;
}
export interface ObserverStats {
  projection_interval_ms: number;
  event_capacity: number;
  updates: number;
  events_total: number;
  events_dropped: number;
  snapshot_writes: number;
  write_calls: number;
  bytes_written: number;
  errors: number;
  pending_updates: number;
  last_error: string | null;
  last_failure_at: string | null;
  last_published_at: string | null;
  last_publish_latency_ms: number;
  max_publish_latency_ms: number;
  legacy_write_estimate: number;
}
export interface Snapshot {
  schema_version: number;
  telemetry?: ObserverStats;
  continuous?: boolean;
  frontier?: {
    pending_domains: number;
    discovered_domains: number;
    capacity: number;
  };
  run_id: string;
  pid: number;
  updated_at: string;
  started_at: string;
  finished_at: string | null;
  phase: "running" | "idle" | "completed" | "stopped" | "failed";
  user_agent: string;
  workers: number;
  min_crawl_delay_ms: number;
  pages_per_domain: number;
  counters: ExplicitCounters & {
    downloaded: number;
    failed: number;
    skipped: number;
    attempted: number;
    bytes: number;
    sites: number;
    completed_sites: number;
  };
  sites: Site[];
  recent_events: CrawlEvent[];
  warc: {
    directory: string;
    files: number;
    bytes: number;
    current_file: string | null;
  };
  history: { at: string; downloaded: number; failed: number; bytes: number }[];
}
export interface StatusResponse {
  snapshot: Snapshot;
  meta: { read_at: string; modified_at: string; process_alive: boolean | null };
}
