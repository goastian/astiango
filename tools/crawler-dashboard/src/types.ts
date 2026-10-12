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

export interface FrontierDbStats {
  site_identity_version?: number;
  identity_mapped_urls?: number;
  identity_migration_complete?: boolean;
  schema_version: number; identity_version: number; cache_bytes: number; batch_size: number;
  urls: number; pending_urls: number; leased_urls: number; committed_urls: number; terminal_urls: number;
  known_sites: number; pending_sites: number; active_sites: number; outbox_entries: number; commits: number;
  transactions: number; recovered_leases: number; reconciled_captures: number; migrated_jobs: number; migration_complete: boolean;
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
export interface TransportStats {
  max_active_per_ip?: number;
  active_ip_reservations?: number;
  ip_active_peak?: number;
  ip_waits?: number;
  requests_started: number;
  headers_received: number;
  request_failures: number;
  request_timeouts: number;
  connect_timeouts?: number;
  header_timeouts?: number;
  total_timeouts?: number;
  bodies_completed: number;
  body_failures: number;
  body_timeouts: number;
  bodies_dropped: number;
  bytes_received: number;
  client_slots_total: number;
  client_slots_active: number;
  client_slots_idle: number;
  client_slots_peak: number;
  max_client_slots: number;
  max_slots_per_origin: number;
  clients_created: number;
  clients_reused: number;
  clients_evicted: number;
  pool_waits: number;
  pool_wait_ms: number;
  dns_cache_entries: number;
  dns_cache_hits: number;
  dns_negative_hits: number;
  dns_cache_misses: number;
  dns_coalesced: number;
  dns_lookups: number;
  dns_failures: number;
  dns_rejected: number;
  dns_inflight: number;
  dns_inflight_peak: number;
  dns_blocking_inflight?: number;
  dns_blocking_inflight_peak?: number;
  dns_lookup_ms: number;
}
export interface BodyStats {
  bodies_started: number;
  bodies_admitted: number;
  bodies_completed: number;
  body_failures: number;
  rejected_status: number;
  rejected_mime: number;
  rejected_content_length: number;
  rejected_encoding: number;
  wire_limit_exceeded: number;
  decoded_limit_exceeded: number;
  stream_failures: number;
  allocation_failures: number;
  idle_timeouts: number;
  total_timeouts: number;
  budget_timeouts: number;
  budget_waits: number;
  budget_wait_ms: number;
  wire_bytes_received: number;
  decoded_bytes_produced: number;
  active_bodies: number;
  reserved_bytes: number;
  peak_active_bodies: number;
  peak_reserved_bytes: number;
  max_inflight_bodies: number;
  max_reserved_bytes: number;
}
export interface Snapshot {
  scheduler?: SchedulerStats;
  robots?: RobotsStats;
  frontier_db?: FrontierDbStats;
  spool?: SpoolStats;
  pipeline?: PipelineStats;
  extraction?: ExtractionStats;
  schema_version: number;
  telemetry?: ObserverStats;
  transport?: TransportStats;
  body?: BodyStats;
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

export interface SchedulerStats {
  mode: "fair_turns" | "long_jobs";
  requests_admitted: number; requests_deferred: number; wait_ms: number;
  active_requests: number; active_peak: number; tracked_origins: number;
  origin_limit: number; site_limit: number; origin_overflows: number;
}
export interface RobotsStats {
  hits: number; misses: number; single_flight_waits: number; downloads: number;
  evictions: number; unreachable: number; unavailable: number; disallowed: number;
  entries: number; capacity: number; ttl_seconds: number; failure_ttl_seconds: number;
}

export interface SpoolStats {
  capture_records?: number; captured_payload_bytes?: number; decoded_payload_bytes?: number;
  noindex_records?: number; nofollow_records?: number;
  records_durable: number; records_recovered: number;
  segments_total: number; segments_open: number; segments_pending: number; segments_published: number;
  spool_bytes: number; max_spool_bytes: number; max_segments: number; segment_bytes: number; gzip_level: number;
  group_commits: number; sync_calls: number; append_calls: number; rotations: number;
  upload_attempts: number; upload_errors: number; upload_buffer_bytes: number; upload_buffer_peak_bytes: number;
  quota_rejections: number; errors: number; reference_pins: number; gc_blocked: number; gc_deleted: number;
  last_error: string | null;
}

export interface StageStats {
  workers: number; max_items: number; max_bytes: number;
  items: number; bytes: number; running: number; queued: number;
  peak_items: number; peak_bytes: number; peak_running: number; peak_queued: number;
  submitted: number; completed: number; cancelled: number; failed: number;
  admission_waits: number; admission_wait_us: number; queue_wait_us: number; work_us: number;
  queue_timeouts: number; input_rejections: number;
}
export interface PipelineStats {
  admission: StageStats; parse: StageStats; network_setup: StageStats; storage: StageStats; control: StageStats; batch_queue: StageStats;
  accepting: boolean; runtime_lag_samples: number; runtime_lag_window_samples: number;
  runtime_lag_p99_us: number; runtime_lag_max_us: number; runtime_sample_interval_ms: number;
  batch_segment_bytes: number; batch_segment_items: number;
}

export interface ExtractionStats {
  documents: number;
  dom_parses: number;
  failures: number;
  anchors_visited: number;
  discovery_links: number;
  internal_links: number;
  pages_at_discovery_limit: number;
  pages_at_internal_limit: number;
  traversals_stopped: number;
  wall_us: number;
  cpu_us: number;
  cpu_samples: number;
  max_links_per_projection: number;
}
export interface StatusResponse {
  snapshot: Snapshot;
  meta: { read_at: string; modified_at: string; process_alive: boolean | null };
}
