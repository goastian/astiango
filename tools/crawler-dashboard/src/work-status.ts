import type { IndexingSnapshot, Snapshot, WebgraphResponse } from "./types";

export function consumerActivity(value: IndexingSnapshot | WebgraphResponse["snapshot"] | undefined, crawler?: Snapshot) {
  if (!value) return { label: "State unavailable or stale", detail: "State unavailable or stale", warning: true };
  const active: Record<string, string> = { indexing: "Indexing", extracting: "Extracting links", publishing: "Preparing generation", centrality: "Calculating scores", draining: "Draining", stopped: "Stopped" };
  if (active[value.phase]) return { label: active[value.phase], detail: value.phase === "publishing" ? "Preparing generation" : "Processing sealed segment", warning: value.phase === "stopped" };
  const reasons: Record<string, string> = { output_quota: "Output quota reached", disk_reserve: "Disk reserve reached", index_pressure: "Index resource pressure", crawler_pressure: "Crawler resource pressure", source_unavailable: "Input spool unavailable", pilot_limit: "Pilot shard limit reached" };
  if (value.phase === "blocked") return { label: "Backpressure", detail: reasons[value.reason] ?? value.reason, warning: true };
  const indexing = value.component === "indexer";
  const pending = indexing ? value.totals.shards_pending + (value.totals.shards_deferred ?? 0) : value.totals.pending;
  const failures = indexing ? value.totals.shards_failed : value.totals.failed;
  const exhausted = (indexing ? value.shards : value.inputs).some(row => row.state === "deferred" && (row.attempts ?? 0) >= 3);
  if (exhausted || (!pending && failures > 0)) return { label: "Recovery required", detail: "Retained inputs need recovery", warning: true };
  if (pending > 0) return { label: "Awaiting retry", detail: "Deferred inputs retain their captures", warning: true };
  if (crawler?.phase === "backpressure") return { label: "Producer blocked", detail: crawler.resources?.reason === "spool_full" ? "Crawler segment quota reached" : "Crawler resource pressure", warning: true };
  if (value.source.open > 0) return { label: "Keeping up", detail: "All closed input consumed; next segment is being captured", warning: false };
  return { label: "Caught up", detail: "All eligible sealed segments consumed", warning: false };
}
