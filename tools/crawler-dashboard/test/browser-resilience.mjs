import assert from "node:assert/strict";
import { readFile, writeFile, mkdir } from "node:fs/promises";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import { isBenchmarkReport } from "../benchmark-report.mjs";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const base = "http://127.0.0.1:3101";
const report = JSON.parse(await readFile(resolve(root,"docs/crawler-c14-c20-resultados-2026-10-12.json"),"utf8"));
assert.equal(isBenchmarkReport(report),true); assert.equal(report.acceptance.passed,true);
const { chromium } = await import(new URL("../../../frontend/node_modules/playwright/index.mjs",import.meta.url).href);
const browser = await chromium.launch({headless:true,executablePath:"/usr/bin/chromium",args:["--no-sandbox"]});
try {
  const page = await browser.newPage({viewport:{width:1440,height:1080}});
  const errors=[];page.on("pageerror",error=>errors.push(String(error)));
  await page.route("**/*",route=>new URL(route.request().url()).origin===base ? route.continue() : route.abort());
  await page.goto(base);
  const status = await (await page.request.get(`${base}/api/status`)).json();
  assert.equal(status.meta.process_alive,true);
  assert.equal(status.snapshot.phase,"running");
  assert.ok(Number.isInteger(status.snapshot.frontier_db.retry_deferred));
  assert.ok(status.snapshot.counters.downloaded > 0);
  assert.ok(!JSON.stringify(status.snapshot.sites).includes("resilience.test"));
  await page.getByText("Recuperación, descubrimiento y revalidación",{exact:true}).waitFor();
  await page.getByRole("button",{name:"Liberar URL",exact:true}).waitFor();
  await page.getByRole("link",{name:"Banco C14–C20 · local",exact:true}).click();
  const panel=page.getByRole("region",{name:"Banco C14–C20",exact:true});
  await panel.waitFor();
  assert.equal(await panel.locator("tbody tr").count(),report.scenarios.length);
  const download=page.waitForEvent("download");
  await page.getByRole("button",{name:"Descargar C14–C20",exact:true}).click();
  assert.deepEqual(JSON.parse(await readFile(await (await download).path(),"utf8")),report);
  assert.deepEqual(errors,[]);
  const output=resolve(root,"data/crawler-resilience-bench/dashboard"); await mkdir(output,{recursive:true});
  await page.screenshot({path:resolve(output,"central-desktop.png"),fullPage:true});
  await page.setViewportSize({width:390,height:844});
  await page.screenshot({path:resolve(output,"central-mobile.png"),fullPage:true});
  const evidence={passed:true,url:base,crawler_pid:status.snapshot.pid,run_id:status.snapshot.run_id,
    downloaded:status.snapshot.counters.downloaded,frontier:status.snapshot.frontier_db,
    report_kind:report.kind,scenarios:report.scenarios.length,page_errors:errors,screenshots:output};
  await writeFile(resolve(root,"docs/crawler-c14-c20-dashboard-verificacion-2026-10-12.json"),JSON.stringify(evidence,null,2)+"\n");
  console.log(JSON.stringify({passed:true,url:base,crawler_pid:evidence.crawler_pid,downloaded:evidence.downloaded,scenarios:evidence.scenarios}));
} finally { await browser.close(); }
