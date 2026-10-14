import assert from "node:assert/strict";
import { mkdtemp, writeFile, rm, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { createDashboardServer } from "../server.mjs";

const root = resolve(new URL("../../..", import.meta.url).pathname);
const base = process.env.INDEXING_LIVE_URL || "http://127.0.0.1:3101";
const live = await (await fetch(`${base}/api/indexing`)).json();
assert.equal(live.meta.process_alive, true);
assert.ok(live.snapshot.totals.indexed_documents > 0);
const directory = await mkdtemp(join(tmpdir(), "indexing-api-"));
const path = join(directory, "indexing.json");
const server = createDashboardServer({ indexingPath: path, statePath: join(root, "data/crawler-local/status.json") });
await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
const api = `http://127.0.0.1:${server.address().port}`;
try {
  assert.equal((await fetch(`${api}/api/indexing`)).status, 503);
  await writeFile(path, JSON.stringify(live.snapshot));
  const clients = await Promise.all(Array.from({ length: 10 }, () => fetch(`${api}/api/indexing`).then(response => response.json())));
  assert.ok(clients.every(value => value.snapshot.run_id === live.snapshot.run_id));
  const cached = await (await fetch(`${api}/api/indexing`)).json();
  assert.equal(cached.meta.cache.reads, 1);
  assert.ok(cached.meta.cache.hits >= 10);
  const paginated = await (await fetch(`${api}/api/indexing/shards?page_size=2&status=completed`)).json();
  assert.ok(paginated.items.length <= 2 && paginated.total > 0);
  assert.ok(paginated.items.every(value => value.state === "completed"));
  assert.equal((await fetch(`${api}/api/indexing/shards?page_size=129`)).status, 400);
  await writeFile(path, JSON.stringify({ ...live.snapshot, schema_version: 999 }));
  assert.equal((await fetch(`${api}/api/indexing`)).status, 503);
  assert.equal((await fetch(`${api}/api/status`)).status, 200);
  await writeFile(path, "x".repeat(256 * 1024 + 1));
  assert.equal((await fetch(`${api}/api/indexing`)).status, 503);
  await writeFile(path, JSON.stringify(live.snapshot));
  assert.equal((await fetch(`${api}/api/indexing`)).status, 200);
} finally {
  server.closeAllConnections();
  await new Promise(resolve => server.close(resolve));
  await rm(directory, { recursive: true, force: true });
}

const { chromium } = await import(new URL("../../../frontend/node_modules/playwright/index.mjs", import.meta.url).href);
const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_PATH || "/usr/bin/chromium", args: ["--no-sandbox", "--renderer-process-limit=2"] });
const errors = [];
const languages = ["en", "de", "pt", "es", "it", "fr", "zh-CN"];
const output = join(root, "data/indexer-shared");
await mkdir(output, { recursive: true });
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1100 } });
  page.on("pageerror", error => errors.push(String(error)));
  await page.route("**/*", route => new URL(route.request().url()).origin === new URL(base).origin ? route.continue() : route.abort());
  for (const locale of languages) {
    await page.goto(`${base}/?ui_lang=${locale}#/indexing/overview`);
    await page.locator(".indexing-cards strong").first().waitFor();
    await page.waitForFunction(language => document.documentElement.lang === language, locale);
    assert.equal(await page.locator("html").getAttribute("lang"), locale);
    assert.equal(await page.locator(".indexing-cards strong").count(), 4);
    assert.equal(await page.locator(".chart-grid svg").count(),3);
    assert.equal(await page.locator(".diagnostic-grid .bars").count(),2);
    assert.ok(await page.locator(".indexing-cards strong").first().innerText());
    const size = await page.evaluate(() => ({ scroll: document.documentElement.scrollWidth, width: innerWidth }));
    assert.ok(size.scroll <= size.width + 1);
    if (locale === "en") await page.screenshot({ path: join(output, "indexing-desktop.png"), fullPage: true });
    await page.goto(`${base}/?ui_lang=${locale}#/indexing/shards`);
    await page.locator(".indexing-content tbody tr").first().waitFor();
    assert.ok(await page.locator(".indexing-content tbody tr").count() <= 10);
    await page.goto(`${base}/?ui_lang=${locale}#/indexing/configuration`);
    await page.locator(".indexing-content .metrics dd").first().waitFor();
    assert.equal(await page.locator(".indexing-content .metrics dd").count(), 8);
  }
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto(`${base}/?ui_lang=es#/indexing/overview`);
  await page.locator(".indexing-cards strong").first().waitFor();
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1));
  await page.screenshot({ path: join(output, "indexing-mobile.png"), fullPage: true });
  await page.locator(".menu-button").click();
  await page.locator("dialog[open] a[href='#/crawler/overview']").click();
  await page.locator("main[data-view='crawler-overview']").waitFor();
  const crawl = await (await fetch(`${base}/api/status`)).json();
  assert.equal(crawl.meta.process_alive, true);
  assert.deepEqual(errors, []);
  const finalIndex=await (await fetch(`${base}/api/indexing`)).json();
  assert.ok(finalIndex.snapshot.runtime.stages.tokenization.cpu_ms>0);
  assert.ok(finalIndex.snapshot.publication.cross_shard_dedup);
  assert.ok(finalIndex.snapshot.history.length>1);
  const report = { real_native_stages:true, svg_charts:3, decision_charts:2, final_index_documents:finalIndex.snapshot.totals.indexed_documents, version: 1, passed: true, browser_locales: languages, browser_errors: errors,
    mobile_overflow: false, crawler_alive: true, real_index_documents: live.snapshot.totals.indexed_documents,
    views: ["overview", "shards", "configuration"], paginated_filtered_api: true, independent_state_failure: true,
    bounded_snapshot: true, concurrent_readers: 10, shared_cache_reads: 1, api_mutations_added: false };
  await writeFile(join(root, "docs/indexador-i01-i05-dashboard-verificacion-2026-10-13.json"), JSON.stringify(report, null, 2) + "\n");
  console.log(JSON.stringify(report));
} finally { await browser.close(); }
