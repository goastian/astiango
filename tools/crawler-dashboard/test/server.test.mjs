import assert from "node:assert/strict";
import { test } from "node:test";
import { mkdtemp, mkdir, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { request } from "node:http";
import { createDashboardServer } from "../server.mjs";

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
