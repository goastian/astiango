import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtemp, mkdir, writeFile, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createDashboardServer } from "../server.mjs";
import { transportReport } from "./transport-report.mjs";
import { bodyReport } from "./body-fixture.mjs";

const metric = { median: 12, min: 10, max: 14, mad: 2 };
function report() {
  return {
    schema_version: 1,
    kind: "crawler-capacity-benchmark",
    generated_at: "2026-10-11T12:00:00Z",
    configuration: { repetitions: 3, origins: [2, 32, 128] },
    scenarios: [
      {
        name: "adversarial",
        origins: 2,
        observer: false,
        repetitions: 3,
        metrics: {
          duration_ms: metric,
          requests: metric,
          stages_ms: { dns: metric },
        },
        requests_match: true,
      },
    ],
    comparisons: [
      {
        name: "adversarial",
        origins: 2,
        duration_overhead_pct: -1.2,
        requests_equal: true,
        saved_pages_equal: true,
      },
    ],
    acceptance: { passed: true },
  };
}

async function setup(t, enabled = true) {
  const directory = await mkdtemp(
    join(tmpdir(), "crawler-benchmark-dashboard-"),
  );
  const distDirectory = join(directory, "dist");
  const statePath = join(directory, "status.json");
  const benchmarkPath = join(directory, "report.json");
  await mkdir(distDirectory);
  await writeFile(join(distDirectory, "index.html"), "<h1>Crawler</h1>");
  const server = createDashboardServer({
    statePath,
    benchmarkPath: enabled ? benchmarkPath : null,
    distDirectory,
  });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  t.after(async () => {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  });
  return { base, statePath, benchmarkPath };
}

test("benchmark is opt-in even if a report exists, without exposing paths", async (t) => {
  const { base, benchmarkPath } = await setup(t, false);
  await writeFile(benchmarkPath, JSON.stringify(report()));
  const response = await fetch(`${base}/api/benchmarks`);
  assert.equal(response.status, 404);
  assert.deepEqual(await response.json(), {
    error: "El banco local no está habilitado en este dashboard.",
    code: "BENCHMARK_DISABLED",
  });
  assert.equal((await fetch(`${base}/report.json`)).status, 404);
});

test("benchmark and crawler documents are independent and read-only", async (t) => {
  const { base, statePath, benchmarkPath } = await setup(t);
  const snapshot = {
    schema_version: 1,
    pid: process.pid,
    sites: [{ host: "production.example" }],
    counters: { downloaded: 91 },
  };
  const benchmark = report();
  await writeFile(statePath, JSON.stringify(snapshot));
  await writeFile(benchmarkPath, JSON.stringify(benchmark));
  const response = await fetch(`${base}/api/benchmarks`);
  const result = await response.json();
  assert.equal(response.status, 200);
  assert.equal(response.headers.get("cache-control"), "no-store");
  assert.deepEqual(result.report, benchmark);
  assert.ok(Number.isFinite(Date.parse(result.meta.modified_at)));
  assert.equal("snapshot" in result, false);
  const status = await (await fetch(`${base}/api/status`)).json();
  assert.deepEqual(status.snapshot, snapshot);
  assert.equal("report" in status, false);
  assert.equal(await readFile(statePath, "utf8"), JSON.stringify(snapshot));
  assert.equal(
    (await fetch(`${base}/api/benchmarks`, { method: "POST", body: "{}" }))
      .status,
    405,
  );
  assert.equal(
    await readFile(benchmarkPath, "utf8"),
    JSON.stringify(benchmark),
  );
});

test("missing or invalid benchmark reports do not interrupt production status", async (t) => {
  const { base, statePath, benchmarkPath } = await setup(t);
  await writeFile(
    statePath,
    JSON.stringify({
      schema_version: 1,
      sites: [],
      counters: { downloaded: 3 },
    }),
  );
  const missing = await fetch(`${base}/api/benchmarks`);
  assert.equal(missing.status, 503);
  assert.equal((await missing.json()).code, "BENCHMARK_UNAVAILABLE");
  for (const source of [
    "{broken",
    "null",
    JSON.stringify({ schema_version: 1, sites: [], counters: {} }),
  ]) {
    await writeFile(benchmarkPath, source);
    const response = await fetch(`${base}/api/benchmarks`);
    assert.equal(response.status, 503);
    assert.equal((await response.json()).code, "BENCHMARK_INVALID");
    assert.equal((await fetch(`${base}/api/status`)).status, 200);
  }
  await writeFile(benchmarkPath, JSON.stringify(report()));
  assert.equal((await fetch(`${base}/api/benchmarks`)).status, 200);
});

test("incompatible distributions and scenario identities are rejected", async (t) => {
  const { base, benchmarkPath } = await setup(t);
  const invalid = [
    (value) => {
      value.schema_version = 2;
    },
    (value) => {
      value.scenarios[0].metrics.duration_ms = { ...metric, median: 99 };
    },
    (value) => {
      value.scenarios[0].metrics.stages_ms.dns = null;
    },
    (value) => {
      value.scenarios[0].origins = -2;
    },
    (value) => {
      value.scenarios[0].observer = "unknown";
    },
    (value) => {
      value.scenarios[0].observer_mode = "unknown";
    },
    (value) => {
      value.scenarios[0].metrics.observer_stats = { write_calls: { ...metric, median: -1 } };
    },
    (value) => {
      value.comparisons[0].write_calls_reduction_pct = { ...metric, mad: -1 };
    },
    (value) => {
      value.scenarios[0].repetitions = 0;
    },
    (value) => {
      value.scenarios[0].requests_match = "yes";
    },
    (value) => {
      value.generated_at = "invalid";
    },
    (value) => {
      value.acceptance.passed = "true";
    },
    (value) => {
      value.comparisons[0].requests_equal = undefined;
    },
    (value) => {
      value.scenarios[0].runs = [{ errors: { other: -1 } }];
    },
    (value) => {
      value.scenarios[0].runs = [{ server: { requests: "12" } }];
    },
  ];
  for (const mutate of invalid) {
    const value = report();
    mutate(value);
    await writeFile(benchmarkPath, JSON.stringify(value));
    const response = await fetch(`${base}/api/benchmarks`);
    assert.equal(response.status, 503, mutate.toString());
    assert.equal((await response.json()).code, "BENCHMARK_INVALID");
  }
});

test("C03 reports preserve both observer modes and signed measured reductions", async (t) => {
  const { base, benchmarkPath } = await setup(t);
  const value = report();
  value.configuration.acceptance_profile = "c03";
  value.configuration.observer_modes = ["legacy_sync", "async"];
  const scenario = value.scenarios[0];
  value.scenarios = ["legacy_sync", "async"].map((mode) => ({
    ...scenario, observer: true, observer_mode: mode,
    metrics: { ...scenario.metrics, observer_stats: { write_calls: metric, updates: metric } },
    runs: [{ repetition: 1, observer_mode: mode, observer_stats: { write_calls: 12 } }],
  }));
  Object.assign(value.comparisons[0], {
    behavior_equivalent: false,
    write_calls_reduction_pct: { median: -2, min: -3, max: -1, mad: 1 },
    writes_per_update_reduction_pct: { median: 95, min: 94, max: 96, mad: 1 },
  });
  await writeFile(benchmarkPath, JSON.stringify(value));
  const response = await fetch(`${base}/api/benchmarks`);
  assert.equal(response.status, 200);
  assert.deepEqual((await response.json()).report, value);
  assert.equal((await fetch(`${base}/api/status`)).status, 503);
});

test("C04 preserves protocol and transport identities with actual server connections", async (t) => {
  const { base, benchmarkPath } = await setup(t);
  const report = transportReport();
  await writeFile(benchmarkPath, JSON.stringify(report));
  const response = await fetch(`${base}/api/benchmarks`);
  assert.equal(response.status, 200);
  const result = await response.json();
  assert.deepEqual(result.report, report);
  assert.equal(result.report.summaries.length, 4);
  assert.equal(result.report.runs[3].server.connections, 2);
  assert.equal(result.report.runs[3].transport_stats.clients_created, 999);
});

test("C04 rejects malformed measurements and duplicated transport identities", async (t) => {
  const { base, benchmarkPath } = await setup(t);
  for (const mutate of [
    (value) => { value.configuration.acceptance_profile = "c03"; },
    (value) => { value.summaries[0].mode = "unknown"; },
    (value) => { value.summaries[0].scheme = "file"; },
    (value) => { value.summaries[0].metrics.connections_per_page.median = -1; },
    (value) => { value.summaries.push(structuredClone(value.summaries[0])); },
    (value) => { value.runs[0].server.connections = "24"; },
    (value) => { value.comparisons[0].connections_per_page_reduction_pct.mad = -1; },
  ]) {
    const report = transportReport();
    mutate(report);
    await writeFile(benchmarkPath, JSON.stringify(report));
    const response = await fetch(`${base}/api/benchmarks`);
    assert.equal(response.status, 503, mutate.toString());
    assert.equal((await response.json()).code, "BENCHMARK_INVALID");
  }
});

test("C05 preserves expected rejections and separates server, received and decoded bytes", async (t) => {
  const { base, benchmarkPath } = await setup(t);
  const report = bodyReport();
  await writeFile(benchmarkPath, JSON.stringify(report));
  const response = await fetch(`${base}/api/benchmarks`);
  assert.equal(response.status, 200);
  const result = await response.json();
  assert.deepEqual(result.report, report);
  assert.equal(result.report.summaries[1].expected_outcome, "rejected_mime");
  assert.equal(result.report.summaries[1].passed, true);
  assert.equal(result.report.runs[3].server.body_bytes_written, 32768);
  assert.equal(result.report.runs[3].body_stats.wire_bytes_received, 0);
});

test("C05 rejects malformed body metrics without changing earlier report contracts", async (t) => {
  const { base, benchmarkPath } = await setup(t);
  for (const mutate of [
    (value) => { value.configuration.acceptance_profile = "c04"; },
    (value) => { value.configuration.profile = null; },
    (value) => { value.summaries[0].metrics.body_bytes_received.median = -1; },
    (value) => { value.summaries[0].passed = "yes"; },
    (value) => { value.summaries.push(structuredClone(value.summaries[0])); },
    (value) => { value.runs[0].body_stats.wire_bytes_received = "4096"; },
    (value) => { value.runs[0].outcomes[0].decoded_bytes = -1; },
    (value) => { value.runs[0].server.body_bytes_written = -1; },
  ]) {
    const report = bodyReport();
    mutate(report);
    await writeFile(benchmarkPath, JSON.stringify(report));
    const response = await fetch(`${base}/api/benchmarks`);
    assert.equal(response.status, 503, mutate.toString());
    assert.equal((await response.json()).code, "BENCHMARK_INVALID");
  }
});
