import assert from 'node:assert/strict';
import { chromium } from '../node_modules/playwright/index.mjs';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
const base=process.env.FRONTEND_URL||'http://127.0.0.1:8000';
const root=new URL('../../',import.meta.url);
const token=(await readFile(new URL('data/search-api/control-token',root),'utf8')).trim();
const apiHeaders={Authorization:`Bearer ${token}`,'X-Search-Traffic':'probe'};
const directory=new URL('docs/assets/frontend-integracion-2026-10-15/',root);await mkdir(directory,{recursive:true});
const browser=await chromium.launch({headless:true,executablePath:process.env.CHROMIUM_PATH||'/usr/bin/chromium',args:['--no-sandbox']});
const report={at:new Date().toISOString(),base,passed:false,traffic:'probe; real index, no fixture responses',checks:[],queries:[],screenshots:[]};
const check=(name)=>report.checks.push(name);
try{
 const context=await browser.newContext({viewport:{width:1440,height:1000},extraHTTPHeaders:{'X-Frontend-Probe':token}});
 const page=await context.newPage();const errors=[],legacy=[];
 page.on('pageerror',error=>errors.push(String(error)));
 page.on('request',request=>{if(request.url().includes('/beta/api/'))legacy.push(request.url());});
 const before=(await(await fetch('http://127.0.0.1:3210/status')).json()).snapshot.traffic.probe.complete??0;
 await page.goto(base,{waitUntil:'networkidle'});
 await page.screenshot({path:new URL('home-desktop.png',directory).pathname});report.screenshots.push('home-desktop.png');
 await page.locator('#searchbar').fill('linux');await page.locator('#searchbar').press('Enter');
 await page.locator('.web-result').first().waitFor();await page.waitForLoadState('networkidle');
 const after=(await(await fetch('http://127.0.0.1:3210/status')).json()).snapshot.traffic.probe.complete??0;
 assert.equal(after-before,1,'one native/API request; no hydration duplicate or auxiliaries');check('home submit, real SSR results and exactly one main query');
 const raw=await(await fetch('http://127.0.0.1:3210/v1/web/search?q=linux&count=10',{headers:apiHeaders})).json();
 await writeFile(new URL('linux-payload.json',directory),JSON.stringify(raw,null,2)+'\n');
 const urls=await page.locator('.web-result h2 a').evaluateAll(nodes=>nodes.map(node=>node.getAttribute('href')));
 assert.deepEqual(urls,raw.results.map(row=>row.url));assert.ok(urls.length);check('DOM URLs/order equal the native payload');
 const engine=raw.results[0].engine_snippet.fragments.map(f=>f.text).join('');
 assert.equal((await page.locator('.web-result .snippet').first().textContent()).trim(),engine.trim());check('engine snippet and escaped highlight preserved');
 report.queries.push({query:'linux',generation:raw.generation,results:urls.length,native_cpu_ms:raw.native_cpu_ms,api_wall_ms:raw.wall_ms});
 await page.screenshot({path:new URL('results-desktop.png',directory).pathname});report.screenshots.push('results-desktop.png');
 const next=page.locator('a[rel="next"]');await next.click();await page.waitForURL(/p=2/);await page.locator('.web-result').first().waitFor();
 const secondUrls=await page.locator('.web-result h2 a').evaluateAll(nodes=>nodes.map(n=>n.getAttribute('href')));
 assert.notDeepEqual(secondUrls,urls);await page.goBack();await page.locator('.web-result').first().waitFor();assert.match(page.url(),/q=linux/);check('pagination and history restore query');
 await page.locator('#searchbar').fill('asdfzzzzqxvnomatches987654321');await page.locator('#searchbar').press('Enter');await page.locator('.search-state').waitFor();assert.equal(await page.locator('.web-result').count(),0);check('empty is explicit and contains no demo');
 await page.goto(base+'/search?q=linux&type=images');await page.getByRole('alert').waitFor();assert.equal(await page.locator('.web-result').count(),0);check('unsupported vertical is an explicit error, not fake web results');
 // A/B are both real backend responses. Delay only delivery to verify stale navigation cannot win.
 await page.goto(base+'/search?q=linux');await page.locator('.web-result').first().waitFor();
 await page.route('**/search/__data.json?**',async route=>{
   if(new URL(route.request().url()).searchParams.get('q')==='privacy'){await new Promise(r=>setTimeout(r,350));}
   await route.continue().catch(()=>{});
 });
 await page.locator('#searchbar').fill('privacy');await page.locator('#searchbar').press('Enter');await page.waitForTimeout(50);
 await page.locator('#searchbar').fill('browser');await page.locator('#searchbar').press('Enter');await page.waitForURL(/q=browser/);await page.waitForTimeout(700);
 assert.equal(await page.locator('#searchbar').inputValue(),'browser');check('latest real navigation wins over delayed query');await page.unroute('**/search/__data.json?**');
 for(const code of ['en','de','pt','es','it','fr','zh-CN']){
   await page.goto(`${base}/search?q=linux&ui_lang=${code}`);await page.locator('.web-result').first().waitFor();
   assert.equal(await page.locator('html').getAttribute('lang'),code);
   assert.equal(await page.locator('.language-select select').inputValue(),code);
   assert.ok(await page.locator('.web-result h2 a').count());
 }
 check('seven languages preserve actual indexed content');
 await page.setViewportSize({width:390,height:844});await page.goto(base+'/search?q=linux&ui_lang=es');await page.locator('.web-result').first().waitFor();
 assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false);check('mobile results have no horizontal overflow');
 await page.screenshot({path:new URL('results-mobile.png',directory).pathname});report.screenshots.push('results-mobile.png');
 await page.locator('.menu').click();await page.keyboard.press('Escape');assert.equal(await page.locator('.menu').getAttribute('aria-expanded'),'false');check('mobile menu closes with Escape and returns focus');
 await page.keyboard.press('/');assert.equal(await page.locator('#searchbar').evaluate(n=>n===document.activeElement),true);await page.keyboard.press('Escape');await page.locator('#searchbar').blur();await page.keyboard.press('j');assert.equal(await page.locator('.web-result h2 a').first().evaluate(n=>n===document.activeElement),true);check('keyboard search and result navigation');
 await page.locator('#safe-search').selectOption('true');await page.locator('.safe-filter button').click();await page.waitForURL(/ss=true/);check('SafeSearch has effective request state');
 const noJs=await browser.newContext({javaScriptEnabled:false,extraHTTPHeaders:{'X-Frontend-Probe':token}});const noJsPage=await noJs.newPage();await noJsPage.goto(base+'/search?q=linux&ui_lang=fr');assert.ok(await noJsPage.locator('.web-result').count());await noJs.close();check('actual SSR results without JavaScript');
 assert.deepEqual(legacy,[]);assert.deepEqual(errors,[]);check('no legacy API calls or browser exceptions');
 // Wait for the bounded projection; normal user traffic is not incremented by authenticated probes.
 await page.waitForTimeout(1300);
 const observation=await(await fetch('http://127.0.0.1:3101/api/frontend?traffic=probe')).json();assert.ok(observation.total>0);assert.ok(observation.queries.some(r=>r.query==='linux' && r.generation && r.nativeCpuMs>=0));check('dashboard reconciles probe query, generation, CPU and bytes');
 await page.setViewportSize({width:1440,height:1000});await page.goto('http://127.0.0.1:3101/#/frontend/overview');await page.locator('.frontend-kpis').waitFor();await page.waitForTimeout(700);await page.screenshot({path:new URL('dashboard.png',directory).pathname});report.screenshots.push('dashboard.png');
 const services={};for(const route of ['api/status','api/indexing','api/webgraph','api/search/status']){const r=await(await fetch('http://127.0.0.1:3101/'+route)).json();services[route]={alive:r.meta?.process_alive,updated_at:r.snapshot?.updated_at,phase:r.snapshot?.phase};assert.notEqual(r.meta?.process_alive,false);}
 report.services=services;check('crawler, indexer, webgraph and Search/API remain alive');
 report.passed=true;report.browserErrors=errors;
} finally {
 await browser.close();
 report.sources={};for(const path of ['frontend/src/lib/server/public-search.ts','frontend/src/routes/search/+page.svelte','frontend/src/lib/server/search-observation.ts'])report.sources[path]=createHash('sha256').update(await readFile(new URL(path,root))).digest('hex');
 await writeFile(new URL('docs/frontend-integracion-2026-10-15.json',root),JSON.stringify(report,null,2)+'\n');
 console.log(JSON.stringify(report,null,2));
}
