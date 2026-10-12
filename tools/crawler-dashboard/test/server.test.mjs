import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { request } from "node:http";
import { createDashboardServer } from "../server.mjs";
import { bodyStats } from "./body-fixture.mjs";

async function setup(t) {
  const directory = await mkdtemp(join(tmpdir(), "crawler-dashboard-test-"));
  const distDirectory = join(directory, "dist");
  const statePath = join(directory, "status.json");
  await mkdir(distDirectory);
  await writeFile(join(distDirectory, "index.html"), "<h1>Crawler</h1>");
  await writeFile(join(directory, "private.txt"), "must not be served");
  const server = createDashboardServer({ statePath, distDirectory });
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const address = server.address();
  const base = `http://127.0.0.1:${address.port}`;
  t.after(async () => {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
    await rm(directory, { recursive: true, force: true });
  });
  return { base, statePath };
}

test("health remains available while missing crawler state returns 503", async (t) => {
  const { base } = await setup(t);
  assert.equal((await fetch(`${base}/healthz`)).status, 200);
  const response = await fetch(`${base}/api/status`);
  assert.equal(response.status, 503);
  assert.equal((await response.json()).code, "STATE_UNAVAILABLE");
});

test("status preserves actual snapshot and reports a live process", async (t) => {
  const { base, statePath } = await setup(t);
  const snapshot = {
    schema_version: 1,
    pid: process.pid,
    sites: [],
    counters: { downloaded: 7 },
  };
  await writeFile(statePath, JSON.stringify(snapshot));
  const response = await fetch(`${base}/api/status`);
  const result = await response.json();
  assert.equal(response.status, 200);
  assert.deepEqual(result.snapshot, snapshot);
  assert.equal(result.snapshot.telemetry, undefined);
  assert.equal(result.snapshot.transport, undefined);
  assert.equal(result.snapshot.body, undefined);
  assert.equal(result.meta.process_alive, true);
  assert.equal(response.headers.get("cache-control"), "no-store");
  await writeFile(statePath, "{broken");
  assert.equal((await fetch(`${base}/api/status`)).status, 503);
});

test("static files support conditional requests and reject encoded path traversal", async (t) => {
  const { base } = await setup(t);
  const response = await fetch(base);
  assert.equal(await response.text(), "<h1>Crawler</h1>");
  const cached = await fetch(base, {
    headers: { "If-None-Match": response.headers.get("etag") },
  });
  assert.equal(cached.status, 304);
  for (const path of [
    "/%2e%2e/private.txt",
    "/..%2fprivate.txt",
    "/%5c..%5cprivate.txt",
  ]) {
    const status = await new Promise((resolve, reject) => {
      const req = request(
        { hostname: "127.0.0.1", port: new URL(base).port, path },
        (res) => {
          res.resume();
          resolve(res.statusCode);
        },
      );
      req.on("error", reject);
      req.end();
    });
    assert.equal(status, 403);
  }
});

test("status preserves additive C02 counters and explicit site outcomes", async (t) => {
  const { base, statePath } = await setup(t);
  const snapshot = {
    schema_version: 1,
    pid: process.pid,
    counters: { downloaded: 2, attempted: 4, page_http_attempts: 8, redirect_hops: 3, useful_pages: 2, failed: 0, deferred: 1, cancelled: 0, storage_failed: 1 },
    sites: [{ host: "example.test", status: "storage_failed", page_http_attempts: 8, useful_pages: 2, storage_failed: 1 }],
  };
  await writeFile(statePath, JSON.stringify(snapshot));
  const response = await fetch(`${base}/api/status`);
  assert.equal(response.status, 200);
  assert.deepEqual((await response.json()).snapshot, snapshot);
});

test("stale or degraded telemetry remains separate from process and WARC health", async (t) => {
  const { base, statePath } = await setup(t);
  const earlier = new Date(Date.now() - 60_000).toISOString();
  const snapshot = {
    schema_version: 1, pid: process.pid, phase: "running", updated_at: earlier,
    counters: { downloaded: 91, failed: 0, storage_failed: 0 }, sites: [],
    telemetry: {
      projection_interval_ms: 1000, event_capacity: 100, updates: 240,
      events_total: 250, events_dropped: 150, snapshot_writes: 20,
      write_calls: 22, bytes_written: 2048, errors: 2, pending_updates: 4,
      last_error: "metrics output unavailable", last_failure_at: earlier,
      last_published_at: earlier, last_publish_latency_ms: 30,
      max_publish_latency_ms: 70, legacy_write_estimate: 241,
    },
  };
  await writeFile(statePath, JSON.stringify(snapshot));
  const response = await fetch(`${base}/api/status`);
  const result = await response.json();
  assert.equal(response.status, 200);
  assert.equal(result.meta.process_alive, true);
  assert.deepEqual(result.snapshot, snapshot);
  assert.equal(result.snapshot.counters.storage_failed, 0);
  assert.equal(result.snapshot.telemetry.events_dropped, 150);
});

test("HTTP and DNS measurements remain additive and separate from page outcomes", async (t) => {
  const { base, statePath } = await setup(t);
  const snapshot = {
    schema_version: 1, pid: process.pid, phase: "running", sites: [],
    counters: { downloaded: 91, failed: 0, storage_failed: 0 },
    transport: {
      requests_started: 120, headers_received: 116, request_failures: 4,
      request_timeouts: 2, body_timeouts: 1, max_client_slots: 128, max_slots_per_origin: 4,
      bodies_completed: 100, body_failures: 3, bodies_dropped: 11, bytes_received: 98304,
      client_slots_total: 8, client_slots_active: 2, client_slots_idle: 6,
      client_slots_peak: 10, clients_created: 12, clients_reused: 108,
      clients_evicted: 4, pool_waits: 7, pool_wait_ms: 230,
      dns_cache_entries: 5, dns_cache_hits: 110, dns_negative_hits: 2,
      dns_cache_misses: 8, dns_coalesced: 3, dns_lookups: 5, dns_failures: 1,
      dns_rejected: 2, dns_inflight: 1, dns_inflight_peak: 3, dns_lookup_ms: 50,
      dns_blocking_inflight: 2, dns_blocking_inflight_peak: 3,
    },
  };
  await writeFile(statePath, JSON.stringify(snapshot));
  const response = await fetch(`${base}/api/status`);
  const result = await response.json();
  assert.equal(response.status, 200);
  assert.deepEqual(result.snapshot, snapshot);
  assert.equal(result.meta.process_alive, true);
  assert.equal(result.snapshot.counters.downloaded, 91);
  assert.equal(result.snapshot.counters.storage_failed, 0);
  assert.equal("connections_created" in result.snapshot.transport, false);
  assert.equal(result.snapshot.transport.dns_inflight, 1);
  assert.equal(result.snapshot.transport.dns_blocking_inflight, 2);
});

test("body admission and memory observations do not become WARC failures", async (t) => {
  const { base, statePath } = await setup(t);
  const snapshot = {
    schema_version: 1, pid: process.pid, phase: "running", sites: [],
    counters: { downloaded: 91, failed: 0, storage_failed: 0 }, body: bodyStats(),
  };
  await writeFile(statePath, JSON.stringify(snapshot));
  const response = await fetch(`${base}/api/status`);
  const result = await response.json();
  assert.equal(response.status, 200);
  assert.deepEqual(result.snapshot, snapshot);
  assert.equal(result.meta.process_alive, true);
  assert.equal(result.snapshot.body.allocation_failures, 1);
  assert.equal(result.snapshot.counters.storage_failed, 0);
  assert.equal(result.snapshot.counters.downloaded, 91);
});
