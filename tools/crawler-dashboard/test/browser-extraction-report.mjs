import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createDashboardServer } from "../server.mjs";
import { isBenchmarkReport } from "../benchmark-report.mjs";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const reportPath = resolve(process.env.CRAWLER_EXTRACTION_REPORT || `${root}/data/crawler-extraction-bench/report.json`);
const output = `${root}/data/crawler-c06-benchmark-dashboard`;
const report = JSON.parse(await readFile(reportPath, "utf8"));
assert.equal(isBenchmarkReport(report), true);
assert.equal(report.acceptance.passed, true);
const { chromium } = await import(new URL("../../../frontend/node_modules/playwright/index.mjs", import.meta.url).href);
const server = createDashboardServer({ statePath: `${root}/data/crawler-extraction-bench/no-production-state.json`, benchmarkPath: reportPath });
let browser;
try {
  await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  assert.deepEqual((await (await fetch(`${base}/api/benchmarks`)).json()).report, report);
  assert.equal((await fetch(`${base}/api/status`)).status, 503);
  browser = await chromium.launch({ headless: true, executablePath: process.env.CHROMIUM_PATH || "/usr/bin/chromium", args: ["--no-sandbox"] });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.route("**/*", (route) => new URL(route.request().url()).origin === base ? route.continue() : route.abort());
  await page.goto(base);
  await page.getByRole("link", { name: "Banco C06 · local", exact: true }).click();
  const rows = page.getByRole("table", { name: "Resultados de extracción C06", exact: true }).locator("tbody tr");
  await rows.first().waitFor();
  assert.equal(await rows.count(), report.summaries.length);
  const format = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 3 });
  for (const [index, row] of report.summaries.entries()) {
    const cells = rows.nth(index).locator("td");
    assert.equal(await rows.nth(index).locator("th").textContent(), row.scenario);
    assert.equal(await cells.nth(2).locator("strong").textContent(), format.format(row.metrics.cpu_ms_per_page.median));
    assert.equal(await cells.nth(3).locator("strong").textContent(), `${format.format(row.metrics.allocated_bytes_per_page.median)} B`);
    assert.equal(await cells.nth(5).locator("strong").textContent(), row.mode === "shared" ? "1" : "2");
  }
  await page.getByRole("combobox", { name: "Escenario de extracción para inspeccionar", exact: true }).selectOption("anchors-300000");
  const details = page.getByRole("table", { name: "Repeticiones de extracción C06", exact: true }).locator("tbody tr");
  assert.equal(await details.count(), 6);
  for (let index = 0; index < 6; index++) {
    const row = details.nth(index);
    assert.equal(await row.locator("td").last().textContent(), "512 / 512");
    if (await row.locator("td").first().textContent() === "Compartida")
      assert.equal(await row.locator("td").nth(3).textContent(), "512");
  }
  assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(), "—");
  const downloadEvent = page.waitForEvent("download");
  await page.getByRole("button", { name: "Descargar C06", exact: true }).click();
  const download = await downloadEvent;
  assert.deepEqual(JSON.parse(await readFile(await download.path(), "utf8")), report);
  await mkdir(dirname(output), { recursive: true });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  await page.screenshot({ path: `${output}-desktop.png`, fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  await page.screenshot({ path: `${output}-mobile.png`, fullPage: true });
  assert.deepEqual(errors, []);
  const evidence = { passed: true, checked_at: new Date().toISOString(), report_path: reportPath,
    summaries_verified: report.summaries.length, selected_scenario: "anchors-300000", download_matches_report: true,
    production_snapshot_absent: true, desktop_overflow: false, mobile_overflow: false, page_errors: errors };
  await writeFile(`${output}-verification.json`, JSON.stringify(evidence, null, 2) + "\n");
  console.log(JSON.stringify(evidence));
} finally {
  await browser?.close(); server.closeAllConnections();
  await new Promise((resolve) => server.close(resolve));
}
