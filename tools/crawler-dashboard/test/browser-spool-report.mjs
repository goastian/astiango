import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createDashboardServer } from "../server.mjs";
import { isBenchmarkReport } from "../benchmark-report.mjs";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const reportPath = resolve(process.env.CRAWLER_SPOOL_REPORT || `${root}/data/crawler-spool-bench/report.json`);
const persistentBase = process.env.CRAWLER_SPOOL_URL;
const output = process.env.CRAWLER_SPOOL_OUTPUT || `${root}/data/crawler-c08-${persistentBase ? "persistent-bank" : "benchmark"}-dashboard`;
const report = JSON.parse(await readFile(reportPath, "utf8"));
assert.equal(isBenchmarkReport(report), true);
assert.equal(report.acceptance.passed, true);
const { chromium } = await import(new URL("../../../frontend/node_modules/playwright/index.mjs", import.meta.url).href);
const server = persistentBase ? null : createDashboardServer({ statePath: `${root}/data/crawler-spool-bench/no-production-state.json`, benchmarkPath: reportPath });
let browser;
try {
  if (server) await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = persistentBase || `http://127.0.0.1:${server.address().port}`;
  assert.ok(["127.0.0.1", "localhost"].includes(new URL(base).hostname));
  assert.deepEqual((await (await fetch(`${base}/api/benchmarks`)).json()).report, report);
  const status = await fetch(`${base}/api/status`);
  assert.equal(status.status, persistentBase ? 200 : 503);
  if (persistentBase) {
    const data = await status.json();
    assert.equal(data.meta.process_alive, true);
    assert.ok(data.snapshot.spool);
    if (process.env.CRAWLER_EXPECTED_PID) assert.equal(data.snapshot.pid, Number(process.env.CRAWLER_EXPECTED_PID));
    assert.equal(data.snapshot.sites.some((site) => site.host.includes(".test")), false);
  }
  browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_PATH || "/usr/bin/chromium", args: ["--no-sandbox"] });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.route("**/*", (route) => new URL(route.request().url()).origin === base ? route.continue() : route.abort());
  await page.goto(base);
  await page.getByRole("link", { name: "Banco C08 · local", exact: true }).click();
  const rows = page.getByRole("table", { name: "Resultados del spool C08", exact: true }).locator("tbody tr");
  await rows.first().waitFor();
  assert.equal(await rows.count(), report.summaries.length);
  const format = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 4 });
  for (const [index, row] of report.summaries.entries()) {
    const cells = rows.nth(index).locator("td");
    assert.equal(await rows.nth(index).locator("th").textContent(), row.scenario);
    assert.equal(await cells.nth(2).locator("strong").textContent(), format.format(row.metrics.pages_per_second.median));
    assert.equal(await cells.nth(3).locator("strong").textContent(), format.format(row.metrics.cpu_ms_per_page.median));
  }
  await page.getByRole("combobox", { name: "Escenario del spool para inspeccionar", exact: true }).selectOption("corpus_group_16");
  const details = page.getByRole("table", { name: "Repeticiones del spool C08", exact: true }).locator("tbody tr");
  assert.equal(await details.count(), report.runs.filter((row) => row.scenario === "corpus_group_16").length);
  if (persistentBase) {
    await page.getByRole("button", { name: "Pausar lectura", exact: true }).click();
    const reading = page.waitForResponse((r) => new URL(r.url()).pathname === "/api/status");
    await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
    const data = await (await reading).json();
    const text = new Intl.NumberFormat("es-CO").format(data.snapshot.counters.downloaded);
    await page.locator(".pages-panel .metric-value strong").filter({ hasText: text }).waitFor();
    assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(), text);
    await page.locator(".spool-panel > summary").click();
    const durable = page.locator(".spool-panel dl > div").filter({ has: page.locator("dt", { hasText: "Páginas durables en el spool" }) }).locator("dd");
    assert.equal((await durable.textContent()).trim(), new Intl.NumberFormat("es-CO").format(data.snapshot.spool.records_durable));
    assert.ok(data.snapshot.spool.records_durable >= data.snapshot.counters.downloaded);
  } else {
    assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(), "—");
  }
  const downloadEvent = page.waitForEvent("download");
  await page.getByRole("button", { name: "Descargar C08", exact: true }).click();
  assert.deepEqual(JSON.parse(await readFile(await (await downloadEvent).path(), "utf8")), report);
  await mkdir(dirname(output), { recursive: true });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  await page.screenshot({ path: `${output}-desktop.png`, fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  await page.screenshot({ path: `${output}-mobile.png`, fullPage: true });
  assert.deepEqual(errors, []);
  const evidence = { passed: true, checked_at: new Date().toISOString(), report_path: reportPath,
    summaries_verified: report.summaries.length, selected_scenario: "corpus_group_16", download_matches_report: true,
    production_snapshot_absent: !persistentBase, persistent_url: persistentBase ?? null, desktop_overflow: false, mobile_overflow: false, page_errors: errors };
  await writeFile(`${output}-verification.json`, JSON.stringify(evidence, null, 2) + "\n");
  console.log(JSON.stringify(evidence));
} finally {
  await browser?.close();
  if (server) { server.closeAllConnections(); await new Promise((resolve) => server.close(resolve)); }
}
