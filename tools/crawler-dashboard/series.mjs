import { mkdir, open, readFile, rename, stat } from "node:fs/promises";
import { dirname } from "node:path";
import { randomUUID } from "node:crypto";

export function resourceSeries(path, capacity = 720) {
  let samples = [];
  let initialized = false;
  let pending;
  let lastWrite = 0;
  let error = null;
  async function initialize() {
    if (initialized) return;
    if (path) {
      try {
        const info = await stat(path);
        if (info.size > 512 * 1024) throw new Error("Series exceeds retention budget");
        const data = JSON.parse(await readFile(path, "utf8"));
        if (data.version !== 1 || !Array.isArray(data.samples)) throw new Error("Unsupported series version");
        samples = data.samples.slice(-capacity);
      } catch (failure) { if (failure.code !== "ENOENT") error = failure.message; }
    }
    initialized = true;
  }
  async function observe(snapshot) {
    if (pending) return pending;
    pending = (async () => {
      await initialize();
      if (Date.now() - lastWrite < 5000 || samples.at(-1)?.at === snapshot.updated_at) return;
      const sample = {
        run_id: snapshot.run_id, at: snapshot.updated_at, pages: snapshot.counters.useful_pages ?? snapshot.counters.downloaded,
        http_attempts: snapshot.counters.page_http_attempts ?? null, wire_bytes: snapshot.body?.wire_bytes_received ?? null,
        rss_bytes: snapshot.resources?.rss_bytes ?? null, cpu_percent: snapshot.resources?.process_cpu_percent ?? null,
        disk_written_bytes: snapshot.resources?.process_written_bytes ?? null,
        spool_bytes: snapshot.spool?.spool_bytes ?? null, workers_allowed: snapshot.resources?.workers_allowed ?? null,
      };
      samples.push(sample); samples = samples.slice(-capacity); lastWrite = Date.now();
      if (!path || error) return;
      await mkdir(dirname(path), { recursive: true });
      const temporary = `${path}.${randomUUID()}.tmp`;
      const file = await open(temporary, "wx", 0o600);
      try { await file.writeFile(JSON.stringify({ version: 1, samples })); await file.sync(); }
      finally { await file.close(); }
      await rename(temporary, path);
      const directory = await open(dirname(path), "r");
      try { await directory.sync(); } finally { await directory.close(); }
    })();
    try { await pending; } catch (failure) { error = failure.message; }
    finally { pending = undefined; }
  }
  return { observe, read: () => ({ version: 1, persistent: !!path, capacity, sample_interval_ms: 5000, error, samples }) };
}
