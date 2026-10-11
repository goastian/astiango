export type SiteStatus =
  | "queued"
  | "crawling"
  | "waiting"
  | "completed"
  | "stopped"
  | "failed";
export interface Site {
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
export interface Snapshot {
  schema_version: number;
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
  counters: {
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
