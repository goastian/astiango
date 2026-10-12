import assert from "node:assert/strict";
import { readFile, writeFile } from "node:fs/promises";
import { dirname,resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { createDashboardServer } from "../server.mjs";
import { isBenchmarkReport } from "../benchmark-report.mjs";
const root = resolve(dirname(fileURLToPath(import.meta.url)),"../../..");
const reportPath = `${root}/data/crawler-capture-frontier-bench/report.json`;
const report = JSON.parse(await readFile(reportPath,"utf8"));
assert.equal(isBenchmarkReport(report),true);
const {chromium} = await import(new URL("../../../frontend/node_modules/playwright/index.mjs",import.meta.url).href);
const server = createDashboardServer({statePath:`${root}/data/crawler-scheduler-bench/absent-legacy-state.json`,benchmarkPath:reportPath});
let browser;
try {
  await new Promise(resolve => server.listen(0,"127.0.0.1",resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  browser = await chromium.launch({headless:true,executablePath:"/usr/bin/chromium",args:["--no-sandbox"]});
  const page = await browser.newPage(); const errors = []; page.on("pageerror",e => errors.push(String(e)));
  await page.route("**/*",r => new URL(r.request().url()).origin === base ? r.continue() : r.abort());
  await page.goto(base); await page.getByRole("link",{name:"Banco C09–C10 · local",exact:true}).click();
  const rows = page.getByRole("table",{name:"Resultados C09 C10",exact:true}).locator("tbody tr");
  await rows.first().waitFor(); assert.equal(await rows.count(),report.summaries.length);
  const download = page.waitForEvent("download"); await page.getByRole("button",{name:"Descargar C09–C10",exact:true}).click();
  assert.deepEqual(JSON.parse(await readFile(await (await download).path(),"utf8")),report);
  assert.deepEqual(errors,[]);
  const evidence = {passed:true,report:"C09–C10",summaries_verified:report.summaries.length,download_matches:true,page_errors:errors};
  await writeFile(`${root}/data/crawler-scheduler-bench/browser-legacy-verification.json`,JSON.stringify(evidence,null,2)+"\n");
  console.log(JSON.stringify(evidence));
} finally { await browser?.close(); server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); }
