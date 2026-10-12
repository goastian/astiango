import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import ts from "typescript";

// Use the project's compiler so these tests also work on its supported Node 20.
const source = await readFile(
  new URL("../src/metrics.ts", import.meta.url),
  "utf8",
);
const { outputText } = ts.transpileModule(source, {
  compilerOptions: {
    target: ts.ScriptTarget.ES2022,
    module: ts.ModuleKind.ESNext,
  },
});
const {
  buildHistoryBars,
  summarizeOutcomes,
  summarizeIssues,
  buildSiteOutcomes,
  snapshotAgeMs,
  observerHealth,
} = await import(
  `data:text/javascript;base64,${Buffer.from(outputText).toString("base64")}`
);

const origin = Date.parse("2026-10-11T05:00:00Z");
const at = (seconds) => new Date(origin + seconds * 1000).toISOString();
const sample = (seconds, downloaded = 0, failed = 0, bytes = 0) => ({
  at: at(seconds),
  downloaded,
  failed,
  bytes,
});

test("publication age uses the observer snapshot time and supports legacy documents", () => {
  assert.equal(snapshotAgeMs({ updated_at: at(5), telemetry: { last_published_at: at(55) } }, Date.parse(at(60))), 5000);
  assert.equal(snapshotAgeMs({ updated_at: at(5) }, Date.parse(at(60))), 55000);
  assert.equal(snapshotAgeMs({ updated_at: at(5), telemetry: { last_published_at: null } }, Date.parse(at(60))), 55000);
  assert.equal(snapshotAgeMs({ updated_at: "invalid" }, Date.parse(at(60))), null);
  assert.equal(snapshotAgeMs(undefined, Date.parse(at(60))), null);
  assert.equal(snapshotAgeMs({ updated_at: at(65) }, Date.parse(at(60))), 0);
});

test("observer degradation and recovery never become crawler or WARC failures", () => {
  const stats = { errors: 3, last_error: null, last_publish_latency_ms: 40 };
  assert.deepEqual(observerHealth(stats), { label: "Publicación al día", warning: false });
  assert.deepEqual(observerHealth({ ...stats, last_error: "metrics disk unavailable" }), { label: "Salida de métricas degradada", warning: true });
  assert.deepEqual(observerHealth(stats, { stale: true }), { label: "Publicación sin actualizar", warning: true });
  assert.deepEqual(observerHealth(stats, { readError: true }), { label: "Lectura no disponible", warning: true });
  assert.deepEqual(observerHealth(stats, { stale: true, paused: true }), { label: "Lectura pausada", warning: false });
  assert.deepEqual(observerHealth({ ...stats, last_publish_latency_ms: 5001 }), { label: "Publicación demorada", warning: true });
  assert.deepEqual(observerHealth(undefined), { label: "Sin detalle del observador", warning: false });
  assert.deepEqual(stats, { errors: 3, last_error: null, last_publish_latency_ms: 40 });
});

test("empty history and a lone baseline do not fabricate observations", () => {
  for (const history of [[], [sample(5, 100)]]) {
    const bars = buildHistoryBars(history, at(60), 1, 12);
    assert.equal(bars.length, 12);
    assert.ok(bars.every((bar) => !bar.observed && bar.downloaded === 0));
  }
  assert.deepEqual(buildHistoryBars([], "invalid", 1), []);
  assert.deepEqual(buildHistoryBars([], at(60), 0), []);
});

test("observed zeros differ from missing data and end boundary belongs to the last bucket", () => {
  const bars = buildHistoryBars(
    [
      sample(0, 10),
      sample(5, 10),
      sample(10, 12, 1, 250),
      sample(60, 17, 1, 400),
    ],
    at(60),
    1,
    12,
  );
  assert.equal(bars[0].observed, false);
  assert.equal(bars[1].observed, true);
  assert.equal(bars[1].downloaded, 0);
  assert.equal(bars[2].downloaded, 2);
  assert.equal(bars[2].failed, 1);
  assert.equal(bars[2].bytes, 250);
  assert.equal(
    bars[11].observed,
    false,
    "the fifty-second gap is not attributed to the last bar",
  );
  const edge = buildHistoryBars([sample(55, 4), sample(60, 7)], at(60), 1, 12);
  assert.equal(edge[11].downloaded, 3);
});

test("counter resets, duplicate timestamps and unsorted samples remain bounded", () => {
  const history = [
    sample(10, 2, 1, 12),
    sample(0, 30, 5, 1000),
    sample(5, 1, 0, 5),
    sample(10, 4, 2, 20),
  ];
  const original = structuredClone(history);
  const bars = buildHistoryBars(history, at(60), 1, 12);
  assert.equal(bars[1].downloaded, 0);
  assert.equal(bars[1].failed, 0);
  assert.equal(bars[1].bytes, 0);
  assert.equal(bars[2].downloaded, 3);
  assert.equal(bars[2].failed, 2);
  assert.equal(bars[2].bytes, 15);
  assert.deepEqual(history, original);
});

test("window boundaries exclude changes that partly precede the selected window", () => {
  const bars = buildHistoryBars(
    [sample(-5, 1), sample(0, 8), sample(5, 10), sample(65, 100)],
    at(60),
    1,
  );
  assert.equal(
    bars.reduce((sum, bar) => sum + bar.downloaded, 0),
    2,
  );
});

test("download success excludes skipped URLs and does not infer in-flight requests", () => {
  assert.deepEqual(
    summarizeOutcomes({ downloaded: 8, failed: 2, skipped: 40, attempted: 0 }),
    {
      downloaded: 8,
      failed: 2,
      skipped: 40,
      storageFailed: 0,
      deferred: 0,
      cancelled: 0,
      settled: 10,
      successRate: 80,
    },
  );
  assert.equal(
    summarizeOutcomes({ downloaded: 0, failed: 0, skipped: 4 }).successRate,
    null,
  );
});

test("issue counts cover only retained warnings and errors with robots before inherited HTTP status", () => {
  const event = (level, message, http_status = null, kind = "failed") => ({
    level,
    message,
    http_status,
    kind,
  });
  assert.deepEqual(
    summarizeIssues([
      event("info", "robots.txt"),
      event("warn", "robots.txt deniega la URL", 503, "skipped"),
      event("warn", "URL ya visitada", 503, "skipped"),
      event("error", "failed", 404),
      event("error", "failed", 429),
      event("error", "connection timed out"),
      event("error", "error sending request"),
    ]),
    [
      { key: "http-4xx", label: "Respuestas HTTP 4xx", count: 2 },
      { key: "network", label: "Errores de conexión", count: 1 },
      { key: "robots", label: "Restricciones de robots.txt", count: 1 },
      { key: "skipped", label: "Otras URL omitidas", count: 1 },
      { key: "timeout", label: "Tiempo de espera agotado", count: 1 },
    ],
  );
});

test("site distribution sorts actual outcomes and omits unmeasured sites", () => {
  const sites = [
    { host: "b.test", pages_downloaded: 5, failed: 1, skipped: 2 },
    { host: "a.test", pages_downloaded: 3, failed: 0, skipped: 0 },
    { host: "empty.test", pages_downloaded: 0, failed: 0, skipped: 0 },
  ];
  assert.deepEqual(buildSiteOutcomes(sites, 1), [
    { host: "b.test", downloaded: 5, failed: 1, skipped: 2, deferred: 0, cancelled: 0, storageFailed: 0, total: 8 },
  ]);
  assert.equal(sites.length, 3);
});

test("explicit outcomes keep deferred and cancelled work out of the success denominator", () => {
  assert.deepEqual(
    summarizeOutcomes({ downloaded: 8, failed: 1, skipped: 5, deferred: 40, cancelled: 6, storage_failed: 1 }),
    { downloaded: 8, failed: 1, skipped: 5, deferred: 40, cancelled: 6, storageFailed: 1, settled: 10, successRate: 80 },
  );
  assert.equal(
    summarizeOutcomes({ downloaded: 0, failed: 0, skipped: 0, deferred: 3, cancelled: 2 }).successRate,
    null,
  );
  assert.deepEqual(buildSiteOutcomes([
    { host: "deferred.test", pages_downloaded: 0, failed: 0, skipped: 0, deferred: 3, cancelled: 1, storage_failed: 2 },
  ]), [
    { host: "deferred.test", downloaded: 0, failed: 0, skipped: 0, deferred: 3, cancelled: 1, storageFailed: 2, total: 6 },
  ]);
});

test("explicit outcome kinds determine the cause before retained HTTP status", () => {
  assert.deepEqual(summarizeIssues([
    { level: "error", kind: "storage_failed", message: "disk full", http_status: 503 },
    { level: "warn", kind: "deferred", message: "retry later", http_status: 429 },
    { level: "warn", kind: "cancelled", message: "stop", http_status: 200 },
  ]), [
    { key: "cancelled", label: "Trabajo cancelado", count: 1 },
    { key: "deferred", label: "Trabajo aplazado", count: 1 },
    { key: "storage", label: "Fallos de almacenamiento", count: 1 },
  ]);
});
