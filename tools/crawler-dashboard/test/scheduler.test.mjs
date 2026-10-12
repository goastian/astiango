import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtemp, writeFile, rm } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { isBenchmarkReport } from "../benchmark-report.mjs";
import { createDashboardServer } from "../server.mjs";
const metric = { median: 10, mad: 1, min: 9, max: 11 };
export function schedulerReport() {
  const metrics = Object.fromEntries(["duration_ms", "pages_per_second", "cpu_ms", "rss_peak_bytes", "ready_lag_p99_ms"].map(key => [key, metric]));
  return { kind: "crawler_scheduler_benchmark", schema_version: 1, generated_at: "2026-10-12T12:00:00Z",
    configuration: { repetitions: 3, workers: 2 },
    runs: [2,32,128].flatMap(origins => ["long_jobs","fair_turns"].flatMap(mode => [1,2,3].map(repetition =>
      ({ origins, mode, repetition, duration_ms: 10.125, pages_per_second: 10.25, cpu_ms: 2.625, rss_peak_bytes: 1000, ready_lag_p99_ms: 1,
        server_verification: { requests_match: true, courtesy_violations: 0 } })))),
    summaries: [2,32,128].flatMap(origins => ["long_jobs","fair_turns"].map(mode => ({ origins, mode, repetitions: 3, metrics }))),
    comparisons: [2,32,128].map(origins => ({ origins, speedup: 4, throughput_gain_pct: 300 })),
    acceptance: { passed: true, courtesy_violations: 0, request_sets_match: true } };
}
test("C11–C13 requires all modes, origins, repetitions and finite server-checked metrics", () => {
  assert.equal(isBenchmarkReport(schedulerReport()), true);
  for (const change of [r => r.runs.pop(), r => r.runs[0] = r.runs[1], r => r.summaries[0] = r.summaries[1],
    r => r.comparisons[0] = r.comparisons[1], r => r.summaries[0].metrics = {},
    r => r.runs[0].cpu_ms = Infinity, r => r.runs[0].server_verification.courtesy_violations = 1]) {
    const invalid = structuredClone(schedulerReport()); change(invalid); assert.equal(isBenchmarkReport(invalid), false);
  }
});
test("C09–C10 reports remain supported alongside the scheduler report", () => {
  const metrics = Object.fromEntries(["peak_rss_bytes", "update_cpu_ms", "update_ms", "database_bytes"].map(key => [key, metric]));
  const report = { kind: "crawler_capture_frontier_benchmark", schema_version: 1, generated_at: "2026-10-12T12:00:00Z",
    configuration: { repetitions: 3 }, summaries: [{ urls: 100000, mode: "redb", metrics: structuredClone(metrics) }],
    comparisons: [{ urls: 100000, peak_rss_bytes_reduction_pct: 25, update_cpu_ms_reduction_pct: -5, update_ms_reduction_pct: 40 }],
    acceptance: { passed: true, crash_checks: 30, capture_checks: 3, rss_growth_bytes: 1000 } };
  assert.equal(isBenchmarkReport(report), true);
  report.summaries[0].metrics.update_ms.median = NaN;
  assert.equal(isBenchmarkReport(report), false);
});
test("Scheduler fixtures never populate production counters; an absent snapshot stays unavailable", async () => {
  const directory = await mkdtemp(join(tmpdir(), "scheduler-dashboard-"));
  const statePath = join(directory,"state.json"), benchmarkPath = join(directory,"report.json");
  await writeFile(benchmarkPath, JSON.stringify(schedulerReport()));
  const server = createDashboardServer({ statePath, benchmarkPath });
  try {
    await new Promise(resolve => server.listen(0,"127.0.0.1",resolve));
    const base = `http://127.0.0.1:${server.address().port}`;
    assert.equal((await fetch(`${base}/api/status`)).status, 503);
    assert.deepEqual((await (await fetch(`${base}/api/benchmarks`)).json()).report, schedulerReport());
    const snapshot = { schema_version: 1, sites: [], counters: { downloaded: 17 }, scheduler: { mode: "fair_turns", requests_admitted: 19 }, robots: { downloads: 2 } };
    await writeFile(statePath, JSON.stringify(snapshot));
    assert.deepEqual((await (await fetch(`${base}/api/status`)).json()).snapshot, snapshot);
  } finally { server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); await rm(directory,{recursive:true,force:true}); }
});
