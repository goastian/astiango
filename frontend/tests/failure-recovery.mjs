// Isolated production reader of the same real index. Never stops operational services.
import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { mkdtemp,readFile,writeFile } from 'node:fs/promises';
import { tmpdir, cpus, totalmem, platform, release, availableParallelism } from 'node:os';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { chromium } from '../node_modules/playwright/index.mjs';
const root=new URL('../../',import.meta.url);
const directory=await mkdtemp(join(tmpdir(),'astiango-frontend-integration-'));
const token=(await readFile(new URL('data/search-api/control-token',root),'utf8')).trim();
const browser=await chromium.launch({headless:true,executablePath:'/usr/bin/chromium',args:['--no-sandbox']});
let child;
async function stop(){if(!child || child.exitCode!==null)return;const previous=child;await new Promise(resolve=>{previous.once('exit',resolve);previous.kill('SIGTERM');});child=null;}
async function start(origin){child=spawn(process.execPath,['build/index.js'],{cwd:new URL('../',import.meta.url),env:{...process.env,HOST:'127.0.0.1',PORT:'8001',ORIGIN:'http://127.0.0.1:8001',SEARCH_API_ORIGIN:origin,FRONTEND_OBSERVATION_PATH:join(directory,'status.json')},stdio:'ignore'});for(let i=0;i<60;i++){try{if((await fetch('http://127.0.0.1:8001/')).ok)return;}catch{}await new Promise(r=>setTimeout(r,100));}throw Error('isolated frontend startup failed');}
const report={hardware:{platform:platform(),release:release(),cpu:cpus()[0]?.model,logical_cpus:cpus().length,available_parallelism:availableParallelism(),memory_bytes:totalmem(),scope:'same development host; no enforced browser quota claimed'},at:new Date().toISOString(),passed:false,scope:'isolated production frontend, real upstream; only its own process is restarted',checks:[],baseline:[]};
try{
 await start('http://127.0.0.1:3210');
 const context=await browser.newContext({extraHTTPHeaders:{'X-Frontend-Probe':token},viewport:{width:1440,height:1000}});const page=await context.newPage();
 const cdp=await context.newCDPSession(page);await cdp.send('Performance.enable',{timeDomain:'threadTicks'});
 const cpu=async()=>Object.fromEntries((await cdp.send('Performance.getMetrics')).metrics.map(m=>[m.name,m.value]));
 for(let repetition=1;repetition<=3;repetition++){
  const before=await cpu(),begin=performance.now();await page.goto('http://127.0.0.1:8001/search?q=linux');await page.locator('.web-result').first().waitFor();const firstResultMs=performance.now()-begin;await page.waitForLoadState('networkidle');const after=await cpu();
  report.baseline.push({repetition,first_result_ms:firstResultMs,wall_to_settled_ms:performance.now()-begin,browser_task_cpu_ms:after.TaskDuration*1000,js_heap_bytes:after.JSHeapUsedSize,generation:await page.locator('.results-body').getAttribute('data-generation'),results:await page.locator('.web-result').count(),resources:await page.evaluate(()=>performance.getEntriesByType('resource').map(r=>({name:new URL(r.name).pathname,bytes:r.encodedBodySize,transfer_bytes:r.transferSize,initiator:r.initiatorType}))) });
 }
 report.checks.push('production build serves actual indexed results; three baseline repetitions, no invented speedup');
 for(const width of [360,768,1440]){await page.setViewportSize({width,height:900});assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);}
 report.checks.push('360/768/1440 layout has no horizontal overflow');
 await page.goto('http://127.0.0.1:8001/settings');await page.waitForLoadState('networkidle');await page.locator('#theme').selectOption('theme-dark');await page.waitForFunction(()=>document.documentElement.classList.contains('theme-dark'));await page.goto('http://127.0.0.1:8001/search?q=linux');await page.locator('.web-result').first().waitFor();assert.match(await page.locator('html').getAttribute('class'),/theme-dark/);report.checks.push('dark preference preserves legible results');
 await stop();await start('http://127.0.0.1:9');
 await page.goto('http://127.0.0.1:8001/search?q=linux');await page.getByRole('alert').waitFor();assert.equal(await page.locator('.web-result').count(),0);assert.equal(await page.locator('#searchbar').inputValue(),'linux');
 assert.equal((await fetch('http://127.0.0.1:8001/api/web/search?q=linux')).status,503);report.checks.push('unavailable upstream retains query, explicit error, no fake results');
 await stop();await start('http://127.0.0.1:3210');await page.reload();await page.locator('.web-result').first().waitFor();report.checks.push('configuration rollback restores real search without index rebuild');
 const forbidden=await fetch('http://127.0.0.1:8001/api/web/experience',{method:'POST',headers:{Origin:'http://attacker.invalid','Content-Type':'application/json'},body:'{}'});assert.equal(forbidden.status,403);report.checks.push('cross-origin telemetry rejected');
 report.context={};for(const name of ['frontend/package-lock.json','frontend/src/routes/search/+page.svelte','frontend/src/lib/server/public-search.ts','configs/search/local.toml'])report.context[name]=createHash('sha256').update(await readFile(new URL(name,root))).digest('hex');
 report.passed=true;
}finally{await stop();await browser.close();await writeFile(new URL('docs/frontend-fallos-baseline-2026-10-15.json',root),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify({passed:report.passed,checks:report.checks,baseline:report.baseline.map(({resources,...row})=>({...row,encoded_resource_body_bytes:resources.reduce((n,r)=>n+r.bytes,0),network_resource_bytes:resources.reduce((n,r)=>n+r.transfer_bytes,0)}))},null,2));}
