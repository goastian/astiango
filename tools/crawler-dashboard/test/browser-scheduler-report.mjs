import assert from "node:assert/strict";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createDashboardServer } from "../server.mjs";
import { isBenchmarkReport } from "../benchmark-report.mjs";
const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const reportPath = resolve(process.env.CRAWLER_SCHEDULER_REPORT || `${root}/data/crawler-scheduler-bench/report.json`);
const persistent = process.env.CRAWLER_SCHEDULER_URL;
const output = `${root}/data/crawler-scheduler-bench/browser-${persistent ? "persistent" : "isolated"}`;
const report = JSON.parse(await readFile(reportPath,"utf8"));
assert.equal(isBenchmarkReport(report),true);
assert.equal(report.acceptance.passed,true);
const { chromium } = await import(new URL("../../../frontend/node_modules/playwright/index.mjs",import.meta.url).href);
const server = persistent ? null : createDashboardServer({statePath:`${root}/data/crawler-scheduler-bench/no-production-state.json`,benchmarkPath:reportPath});
let browser;
try {
  if (server) await new Promise(resolve => server.listen(0,"127.0.0.1",resolve));
  const base = persistent || `http://127.0.0.1:${server.address().port}`;
  assert.equal(new URL(base).hostname,"127.0.0.1");
  assert.deepEqual((await (await fetch(`${base}/api/benchmarks`)).json()).report,report);
  assert.equal((await fetch(`${base}/api/status`)).status,persistent ? 200 : 503);
  browser = await chromium.launch({headless:true,executablePath:process.env.CHROMIUM_PATH || "/usr/bin/chromium",args:["--no-sandbox"]});
  const page = await browser.newPage({viewport:{width:1440,height:1000}});
  const errors = []; page.on("pageerror",error => errors.push(String(error)));
  await page.route("**/*",route => new URL(route.request().url()).origin === base ? route.continue() : route.abort());
  await page.goto(base);
  await page.getByRole("link",{name:"Banco C11–C13 · local",exact:true}).click();
  const rows = page.getByRole("table",{name:"Resultados C11 C12 C13",exact:true}).locator("tbody tr");
  await rows.first().waitFor(); assert.equal(await rows.count(),6);
  const format = new Intl.NumberFormat("es-CO",{maximumFractionDigits:2});
  for (const [index,row] of report.summaries.entries()) {
    const cells = rows.nth(index).locator("td");
    assert.equal((await cells.nth(0).textContent()).trim(),format.format(row.origins));
    assert.equal((await cells.nth(2).textContent()).trim(),format.format(row.metrics.pages_per_second.median)+" MAD "+format.format(row.metrics.pages_per_second.mad));
  }
  const download = page.waitForEvent("download");
  await page.getByRole("button",{name:"Descargar C11–C13",exact:true}).click();
  assert.deepEqual(JSON.parse(await readFile(await (await download).path(),"utf8")),report);
  if (persistent) {
    await page.getByRole("button",{name:"Pausar lectura",exact:true}).click();
    const reading = page.waitForResponse(r => new URL(r.url()).pathname === "/api/status");
    await page.getByRole("button",{name:"Actualizar estado ahora",exact:true}).click();
    const data = await (await reading).json();
    assert.equal(data.meta.process_alive,true);
    assert.equal(data.snapshot.scheduler.mode,"fair_turns");
    assert.equal(data.snapshot.frontier_db.site_identity_version,2);
    assert.equal(data.snapshot.frontier_db.identity_migration_complete,true);
    assert.equal(data.snapshot.sites.some(s => s.host.includes(".test")),false);
    await page.locator(".scheduling-panel > summary").click();
    await page.getByRole("region",{name:"Scheduler C12"}).waitFor();
    const admitted = page.locator(".scheduling-panel dl > div").filter({has:page.locator("dt",{hasText:"Solicitudes admitidas"})}).locator("dd");
    assert.equal((await admitted.textContent()).trim(),format.format(data.snapshot.scheduler.requests_admitted));
    assert.ok(data.snapshot.spool.records_durable >= data.snapshot.counters.downloaded);
  } else {
    assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(),"—");
  }
  await mkdir(dirname(output),{recursive:true});
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth),false);
  await page.screenshot({path:`${output}-desktop.png`,fullPage:true});
  await page.setViewportSize({width:390,height:844});
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth),false);
  await page.screenshot({path:`${output}-mobile.png`,fullPage:true});
  assert.deepEqual(errors,[]);
  const evidence = {passed:true,checked_at:new Date().toISOString(),summaries_verified:6,download_matches:true,
    operational_counters_separate:true,persistent_url:persistent || null,desktop_overflow:false,mobile_overflow:false,page_errors:errors};
  await writeFile(`${output}-verification.json`,JSON.stringify(evidence,null,2)+"\n");
  console.log(JSON.stringify(evidence));
} finally {
  await browser?.close();
  if (server) {server.closeAllConnections();await new Promise(resolve => server.close(resolve));}
}
