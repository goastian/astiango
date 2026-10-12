import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createDashboardServer } from "../server.mjs";
import { isBenchmarkReport } from "../benchmark-report.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const reportPath = resolve(process.env.CRAWLER_TRANSPORT_REPORT || join(root, "data/crawler-transport-bench/report.json"));
const output = resolve(process.env.CRAWLER_TRANSPORT_REPORT_OUTPUT || join(root, "data/crawler-c04-benchmark-dashboard"));
const report = JSON.parse(await readFile(reportPath, "utf8"));
assert.equal(report.kind, "crawler_transport_benchmark");
assert.equal(isBenchmarkReport(report), true);
assert.equal(report.acceptance.passed, true);
const { chromium } = await import(process.env.CRAWLER_PLAYWRIGHT_MODULE ||
  new URL("../../../frontend/node_modules/playwright/index.mjs", import.meta.url).href);
const directory = await mkdtemp(join(tmpdir(), "crawler-c04-report-view-"));
const server = createDashboardServer({ statePath: join(directory, "no-crawler-state.json"), benchmarkPath: reportPath });
let browser;
try {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const api = await fetch(`${base}/api/benchmarks`);
  assert.equal(api.status, 200);
  assert.deepEqual((await api.json()).report, report);
  assert.equal((await fetch(`${base}/api/status`)).status, 503);
  browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_PATH || "/usr/bin/chromium", args: ["--no-sandbox"] });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.route("**/*", (route) => new URL(route.request().url()).origin === base ? route.continue() : route.abort());
  await page.goto(base);
  await page.getByRole("link", { name: "Banco C04 · local", exact: true }).click();
  const table = page.getByRole("table", { name: "Resultados del transporte C04", exact: true });
  await table.waitFor();
  assert.equal(await table.locator("tbody tr").count(), report.summaries.length);
  const formatter = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 4 });
  for (const [index, row] of report.summaries.entries()) {
    const rendered = table.locator("tbody tr").nth(index);
    assert.equal(await rendered.locator("th").textContent(), row.scheme.toUpperCase());
    assert.equal(await rendered.locator("td").nth(0).textContent(), String(row.origins));
    assert.equal(await rendered.locator("td").nth(5).locator("strong").textContent(), formatter.format(row.metrics.connections.median));
    assert.equal(await rendered.locator("td").nth(6).locator("strong").textContent(), formatter.format(row.metrics.connections_per_page.median));
  }
  const candidate = report.summaries.findIndex((row) => row.mode === "pooled" && row.origins === 128);
  assert.ok(candidate >= 0);
  await page.getByRole("combobox", { name: "Escenario de transporte para inspeccionar", exact: true }).selectOption(String(candidate));
  const selected = report.summaries[candidate];
  const runs = report.runs.filter((run) => run.scheme === selected.scheme && run.origins === selected.origins && run.mode === selected.mode);
  const details = page.getByRole("table", { name: "Conexiones reales por repetición", exact: true }).locator("tbody tr");
  assert.equal(await details.count(), runs.length);
  for (const [index, run] of runs.entries())
    assert.equal(await details.nth(index).locator("td").nth(2).textContent(), formatter.format(run.server.connections));
  assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(), "—");
  const downloadEvent = page.waitForEvent("download");
  await page.getByRole("button", { name: "Descargar C04", exact: true }).click();
  const download = await downloadEvent;
  assert.deepEqual(JSON.parse(await readFile(await download.path(), "utf8")), report);
  await mkdir(dirname(output), { recursive: true });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  await page.screenshot({ path: `${output}-desktop.png`, fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  await page.screenshot({ path: `${output}-mobile.png`, fullPage: true });
  assert.deepEqual(errors, []);
  const evidence = {
    passed: true, checked_at: new Date().toISOString(), report_path: reportPath,
    report_source_sha256: report.code?.source_sha256, summaries_verified: report.summaries.length,
    selected_scheme: selected.scheme, selected_origins: selected.origins, selected_mode: selected.mode,
    server_connections_verified: runs.map((run) => run.server.connections),
    download_matches_report: true, production_snapshot_absent: true,
    desktop_overflow: false, mobile_overflow: false, page_errors: errors,
    screenshots: [`${output}-desktop.png`, `${output}-mobile.png`],
  };
  await writeFile(`${output}-verification.json`, JSON.stringify(evidence, null, 2) + "\n");
  console.log(JSON.stringify(evidence));
} finally {
  await browser?.close();
  server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
  await rm(directory, { recursive: true, force: true });
}
