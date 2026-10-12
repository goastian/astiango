import assert from "node:assert/strict";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createDashboardServer } from "../server.mjs";
import { isBenchmarkReport } from "../benchmark-report.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const reportPath = resolve(process.env.CRAWLER_BODY_REPORT || join(root, "data/crawler-body-bench/report.json"));
const output = resolve(process.env.CRAWLER_BODY_REPORT_OUTPUT || join(root, "data/crawler-c05-benchmark-dashboard"));
const report = JSON.parse(await readFile(reportPath, "utf8"));
assert.equal(report.kind, "crawler_body_benchmark");
assert.equal(isBenchmarkReport(report), true);
assert.equal(report.acceptance.passed, true);
const { chromium } = await import(process.env.CRAWLER_PLAYWRIGHT_MODULE ||
  new URL("../../../frontend/node_modules/playwright/index.mjs", import.meta.url).href);
const directory = await mkdtemp(join(tmpdir(), "crawler-c05-report-view-"));
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
  await page.getByRole("link", { name: "Banco C05 · local", exact: true }).click();
  const table = page.getByRole("table", { name: "Resultados de admisión y cuerpos C05", exact: true });
  await table.waitFor();
  assert.equal(await table.locator("tbody tr").count(), report.summaries.length);
  const formatter = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 2 });
  for (const [index, row] of report.summaries.entries()) {
    const rendered = table.locator("tbody tr").nth(index);
    assert.equal(await rendered.locator("th").textContent(), row.scenario);
    assert.equal(await rendered.locator("td").nth(0).textContent(), row.expected_outcome);
    assert.equal(await rendered.locator("td").nth(1).textContent(), String(row.repetitions));
    assert.equal(await rendered.locator("td").nth(2).textContent(), row.passed ? "Reglas cumplidas" : "No superada");
    for (const [column, metric] of ["server_body_bytes_written", "body_bytes_received", "body_bytes_decoded"].entries())
      assert.equal(await rendered.locator("td").nth(column + 4).locator("strong").textContent(), `${formatter.format(row.metrics[metric].median)} B`);
  }
  const candidate = report.summaries.findIndex((row) => row.scenario === "mime");
  assert.ok(candidate >= 0, "the actual report includes the rejected MIME scenario");
  await page.getByRole("combobox", { name: "Escenario de cuerpos para inspeccionar", exact: true }).selectOption(String(candidate));
  const selected = report.summaries[candidate];
  const runs = report.runs.filter((run) => run.scenario === selected.scenario);
  const details = page.getByRole("table", { name: "Resultados de cuerpos por repetición", exact: true }).locator("tbody tr");
  assert.equal(await details.count(), runs.length);
  for (const [index, run] of runs.entries()) {
    const cells = details.nth(index).locator("td");
    assert.equal(await cells.nth(0).textContent(), run.outcomes.map((outcome) => `${outcome.path}: ${outcome.cause}`).join(" · "));
    assert.equal(await cells.nth(1).textContent(), `${formatter.format(run.body_stats.wire_bytes_received)} B`);
    assert.equal(await cells.nth(2).textContent(), `${formatter.format(run.body_stats.decoded_bytes_produced)} B`);
    assert.equal(await cells.nth(3).textContent(), `${formatter.format(run.server.body_bytes_written)} B`);
    assert.equal(await cells.nth(4).textContent(), "Reglas cumplidas");
  }
  assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(), "—");
  const downloadEvent = page.waitForEvent("download");
  await page.getByRole("button", { name: "Descargar C05", exact: true }).click();
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
    selected_scenario: selected.scenario,
    bytes_verified: runs.map((run) => ({ received: run.body_stats.wire_bytes_received,
      decoded: run.body_stats.decoded_bytes_produced, server_written: run.server.body_bytes_written })),
    expected_rejection_passed_verification: true,
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
