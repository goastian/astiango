import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile, rm, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { performance } from "node:perf_hooks";
import { createDashboardServer } from "../server.mjs";

const root = resolve(new URL("../../..", import.meta.url).pathname);
const base = process.env.CRAWLER_LIVE_URL || "http://127.0.0.1:3101";
const initial = await (await fetch(`${base}/api/status`)).json();
assert.equal(initial.meta.process_alive, true);
assert.ok(initial.snapshot.resources?.sampled_at);
const directory = await mkdtemp(join(tmpdir(), "crawler-operations-api-"));
const statePath = join(directory,"status.json");
const seriesPath = join(directory,"series.json");
await writeFile(statePath,JSON.stringify(initial.snapshot));
const options={statePath,seriesPath,operationsPath:join(root,"data/crawler-operations/report.json")};
let server=createDashboardServer(options);
await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
let api=`http://127.0.0.1:${server.address().port}`;
let final;
const started=performance.now();
try {
  for(let round=0;round<10;round++) {
    const clients=await Promise.all(Array.from({length:10},async()=>{
      const response=await fetch(`${api}/api/status`);assert.equal(response.status,200);return response.json();
    }));
    for(const observation of clients) assert.equal(observation.snapshot.run_id,initial.snapshot.run_id);
    final=clients.at(-1);
  }
  const status=await (await fetch(`${api}/api/status`)).json();
  assert.equal(status.meta.cache.reads,1);assert.ok(status.meta.cache.hits>=99);
  const page=await (await fetch(`${api}/api/sites?page=1&page_size=2`)).json();
  assert.ok(page.items.length<=2);assert.equal(page.total,initial.snapshot.sites.length);
  assert.equal((await fetch(`${api}/api/sites?page_size=101`)).status,400);
  const metrics=await (await fetch(`${api}/metrics`)).text();
  assert.match(metrics,/crawler_rss_bytes \d+/);assert.equal(metrics.includes("http://"),false);
  const firstSeries=await (await fetch(`${api}/api/series?run_id=${initial.snapshot.run_id}`)).json();
  assert.equal(firstSeries.samples.length,1);
  await new Promise(resolve=>server.close(resolve));
  server=createDashboardServer(options);await new Promise(resolve=>server.listen(0,"127.0.0.1",resolve));
  api=`http://127.0.0.1:${server.address().port}`;
  const restored=await (await fetch(`${api}/api/series?run_id=${initial.snapshot.run_id}`)).json();
  assert.equal(restored.samples.length,1);assert.equal(restored.samples[0].rss_bytes,initial.snapshot.resources.rss_bytes);
  const {chromium}=await import(new URL("../../../frontend/node_modules/playwright/index.mjs",import.meta.url).href);
  const browser=await chromium.launch({headless:true,executablePath:process.env.CHROMIUM_PATH||"/usr/bin/chromium",args:["--no-sandbox"]});
  const output=join(root,"data/crawler-operations");await mkdir(output,{recursive:true});
  const errors=[];const locales=[];
  try {
    const page=await browser.newPage({viewport:{width:1440,height:1100}});
    page.on("pageerror",error=>errors.push(String(error)));
    await page.route("**/*",route=>new URL(route.request().url()).origin===new URL(base).origin?route.continue():route.abort());
    for(const lang of ["en","de","pt","es","it","fr","zh-CN"]){
      await page.goto(`${base}/?ui_lang=${lang}#/crawler/performance`);
      await page.locator(".resource-panel dl").waitFor();
      assert.equal(await page.locator("html").getAttribute("lang"),lang);
      assert.equal(await page.locator(".resource-panel dd").count(),13);
      const width=await page.evaluate(()=>({scroll:document.documentElement.scrollWidth,viewport:innerWidth}));
      assert.ok(width.scroll<=width.viewport+1);
      await page.goto(`${base}/?ui_lang=${lang}#/tools/benchmarks`);
      await page.locator(".operations-panel table").first().waitFor();
      await page.locator(".operations-panel").getByText(lang === "en" ? "Distributed acquisition" : lang === "es" ? "Adquisición distribuida" : lang === "zh-CN" ? "分布式抓取" : lang === "de" ? "Verteilte Erfassung" : lang === "pt" ? "Aquisição distribuída" : lang === "it" ? "Acquisizione distribuita" : "Acquisition distribuée", { exact:true }).waitFor();
      locales.push(lang);
      if(lang==="en")await page.screenshot({path:join(output,"dashboard-performance.png"),fullPage:true});
    }
    await page.setViewportSize({width:390,height:844});
    const mobile=await page.evaluate(()=>({scroll:document.documentElement.scrollWidth,viewport:innerWidth}));assert.ok(mobile.scroll<=mobile.viewport+1);
    await page.screenshot({path:join(output,"dashboard-mobile.png"),fullPage:true});
    assert.deepEqual(errors,[]);
  }finally{await browser.close();}
  const report={version:1,passed:true,run_id:initial.snapshot.run_id,crawler_pid:initial.snapshot.pid,
    api_requests:100,concurrent_clients:10,snapshot_reads:1,cache_reuse_percent:99,
    api_comparison_elapsed_ms:performance.now()-started,pagination:true,persistent_series_restart:true,aggregate_metrics_no_url_labels:true,
    browser_locales:locales,browser_errors:errors,mobile_overflow:false,
    throughput_comparison:"not measured; cache reuse is not crawler throughput"};
  await writeFile(join(output,"dashboard-verification.json"),JSON.stringify(report,null,2)+"\n");console.log(JSON.stringify(report));
}finally{await new Promise(resolve=>server.close(resolve));await rm(directory,{recursive:true,force:true});}
