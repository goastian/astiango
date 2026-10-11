import type { CrawlEvent, Site, Snapshot } from "./types";

export interface HistoryBar {
  start: number;
  end: number;
  downloaded: number;
  failed: number;
  bytes: number;
  observed: boolean;
}

export interface IssueSummary {
  key: string;
  label: string;
  count: number;
}

export interface SiteOutcome {
  host: string;
  downloaded: number;
  failed: number;
  skipped: number;
  total: number;
}

function nonnegative(value: number): number {
  return Number.isFinite(value) ? Math.max(0, value) : 0;
}

/**
 * Counts observed counter changes at the timestamp at which they were recorded.
 * The observer samples every five seconds. Intervals over thirty seconds are
 * gaps, not evidence of zero activity; their unknown timing is never interpolated.
 * The first sample is a baseline, and counter resets never create negative bars.
 */
export function buildHistoryBars(
  history: Snapshot["history"],
  updatedAt: string,
  windowMinutes: number,
  bucketCount = 12,
): HistoryBar[] {
  const end = Date.parse(updatedAt);
  if (
    !Number.isFinite(end) ||
    !Number.isFinite(windowMinutes) ||
    windowMinutes <= 0
  ) {
    return [];
  }
  const count = Number.isFinite(bucketCount)
    ? Math.min(240, Math.max(1, Math.floor(bucketCount)))
    : 12;
  const start = end - windowMinutes * 60_000;
  const bucketWidth = (end - start) / count;
  const bars = Array.from(
    { length: count },
    (_, index): HistoryBar => ({
      start: start + index * bucketWidth,
      end: start + (index + 1) * bucketWidth,
      downloaded: 0,
      failed: 0,
      bytes: 0,
      observed: false,
    }),
  );

  // The last entry wins for duplicate timestamps, without changing caller data.
  const samples = new Map<number, Snapshot["history"][number]>();
  for (const sample of history) {
    const at = Date.parse(sample.at);
    if (Number.isFinite(at) && at <= end) samples.set(at, sample);
  }
  const ordered = [...samples.entries()].sort(([a], [b]) => a - b);
  for (let index = 1; index < ordered.length; index += 1) {
    const [previousAt, previous] = ordered[index - 1];
    const [at, sample] = ordered[index];
    if (previousAt < start || at - previousAt > 30_000) continue;
    const bucketIndex = Math.min(
      count - 1,
      Math.floor((at - start) / bucketWidth),
    );
    const bucket = bars[bucketIndex];
    if (!bucket) continue;
    bucket.observed = true;
    bucket.downloaded += nonnegative(sample.downloaded - previous.downloaded);
    bucket.failed += nonnegative(sample.failed - previous.failed);
    bucket.bytes += nonnegative(sample.bytes - previous.bytes);
  }
  return bars;
}

/** Success is measured only among downloaded pages and recorded failures. */
export function summarizeOutcomes(counters: Snapshot["counters"]) {
  const downloaded = nonnegative(counters.downloaded);
  const failed = nonnegative(counters.failed);
  const skipped = nonnegative(counters.skipped);
  const settled = downloaded + failed;
  return {
    downloaded,
    failed,
    skipped,
    settled,
    successRate: settled > 0 ? (downloaded / settled) * 100 : null,
  };
}

function issueCategory(event: CrawlEvent): [string, string] {
  const message = event.message.toLowerCase();
  if (message.includes("robots.txt"))
    return ["robots", "Restricciones de robots.txt"];
  // A skipped URL can retain a previous HTTP status in the observer snapshot.
  if (event.kind === "skipped") return ["skipped", "Otras URL omitidas"];
  if (event.http_status !== null && event.http_status >= 500) {
    return ["http-5xx", "Respuestas HTTP 5xx"];
  }
  if (event.http_status !== null && event.http_status >= 400) {
    return ["http-4xx", "Respuestas HTTP 4xx"];
  }
  if (/timeout|timed out|tiempo de espera/.test(message)) {
    return ["timeout", "Tiempo de espera agotado"];
  }
  if (/content.?type|content too large|invalid html|contenido/.test(message)) {
    return ["content", "Contenido no compatible"];
  }
  if (
    /dns|connect|tls|ssl|certificate|certificado|network|conexi[oó]n|sending request/.test(
      message,
    )
  ) {
    return ["network", "Errores de conexión"];
  }
  return event.level === "error"
    ? ["other-errors", "Otros errores"]
    : ["other-warnings", "Otras advertencias"];
}

/** Counts only retained warning/error events, never lifetime incidents. */
export function summarizeIssues(events: CrawlEvent[]): IssueSummary[] {
  const grouped = new Map<string, IssueSummary>();
  for (const event of events) {
    if (event.level === "info") continue;
    const [key, label] = issueCategory(event);
    const issue = grouped.get(key);
    if (issue) issue.count += 1;
    else grouped.set(key, { key, label, count: 1 });
  }
  return [...grouped.values()].sort(
    (a, b) => b.count - a.count || a.key.localeCompare(b.key),
  );
}

/** The retained site rows represent the latest job for each domain. */
export function buildSiteOutcomes(sites: Site[], limit = 8): SiteOutcome[] {
  return sites
    .map((site): SiteOutcome => {
      const downloaded = nonnegative(site.pages_downloaded);
      const failed = nonnegative(site.failed);
      const skipped = nonnegative(site.skipped);
      return {
        host: site.host,
        downloaded,
        failed,
        skipped,
        total: downloaded + failed + skipped,
      };
    })
    .filter((site) => site.total > 0)
    .sort((a, b) => b.total - a.total || a.host.localeCompare(b.host))
    .slice(0, Number.isFinite(limit) ? Math.max(0, Math.floor(limit)) : 8);
}
