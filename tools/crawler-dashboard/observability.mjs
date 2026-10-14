import { open, stat } from "node:fs/promises";

export function snapshotCache(path, maximumBytes = 16 * 1024 * 1024, validate = snapshot => snapshot.schema_version === 1 && Array.isArray(snapshot.sites) && snapshot.counters) {
  let cached;
  let pending;
  const counters = { requests: 0, reads: 0, hits: 0, failures: 0 };
  const identity = (info) => `${info.dev}:${info.ino}:${info.size}:${info.mtimeMs}:${info.ctimeMs}`;
  async function get() {
    counters.requests++;
    const info = await stat(path);
    if (cached?.identity === identity(info)) { counters.hits++; return cached; }
    if (pending) { counters.hits++; return pending; }
    pending = (async () => {
      const file = await open(path, "r");
      try {
        const info = await file.stat();
        if (info.size > maximumBytes) throw new Error("Snapshot exceeds size limit");
        const source = await file.readFile("utf8");
        const snapshot = JSON.parse(source);
        if (!validate(snapshot)) {
          const error = new Error("Unsupported crawler snapshot");
          error.code = "STATE_SCHEMA";
          throw error;
        }
        counters.reads++;
        cached = { identity: identity(info), source, snapshot, modifiedAt: info.mtime.toISOString() };
        return cached;
      } finally { await file.close(); }
    })();
    try { return await pending; }
    catch (error) { counters.failures++; throw error; }
    finally { pending = undefined; }
  }
  return { get, counters };
}

export function paginate(items, parameters, fields) {
  const page = Number(parameters.get("page") || 1);
  const pageSize = Number(parameters.get("page_size") || 25);
  if (!Number.isSafeInteger(page) || page < 1 || !Number.isSafeInteger(pageSize) || pageSize < 1 || pageSize > 100) {
    throw new Error("Page must be positive and page_size must be 1..100");
  }
  const query = (parameters.get("q") || "").toLocaleLowerCase("en");
  if (query.length > 512) throw new Error("Filter exceeds 512 characters");
  const status = parameters.get("status");
  const filtered = items.filter((item) => (!status || item.status === status) &&
    (!query || fields.some((field) => String(item[field] ?? "").toLocaleLowerCase("en").includes(query))));
  return { items: filtered.slice((page - 1) * pageSize, page * pageSize), total: filtered.length, page, page_size: pageSize };
}

export function aggregateMetrics(snapshot, cache) {
  const metrics = {
    crawler_pages_durable_total: snapshot.counters.useful_pages ?? snapshot.counters.downloaded,
    crawler_http_attempts_total: snapshot.counters.page_http_attempts,
    crawler_storage_failures_total: snapshot.counters.storage_failed,
    crawler_revalidations_total: snapshot.counters.revalidations,
    crawler_rss_bytes: snapshot.resources?.rss_bytes,
    crawler_memory_limit_bytes: snapshot.resources?.memory_limit_bytes,
    crawler_admission_workers: snapshot.resources?.workers_allowed,
    crawler_spool_bytes: snapshot.spool?.spool_bytes,
    crawler_spool_limit_bytes: snapshot.spool?.max_spool_bytes,
    dashboard_snapshot_reads_total: cache.reads,
    dashboard_snapshot_cache_hits_total: cache.hits,
  };
  return Object.entries(metrics).filter(([, value]) => Number.isFinite(value)).map(([name, value]) => `${name} ${value}\n`).join("");
}
