import assert from 'node:assert/strict';
import { writeFile, mkdir } from 'node:fs/promises';
import { resolve, join } from 'node:path';
import { chromium } from '../../../frontend/node_modules/playwright/index.mjs';
const root=resolve(new URL('../../..',import.meta.url).pathname);
const base=process.env.INDEXING_LIVE_URL||'http://127.0.0.1:3101';
const output=join(root,'data/indexer-shared');await mkdir(output,{recursive:true});
const errors=[];const browser=await chromium.launch({executablePath:'/usr/bin/chromium',args:['--no-sandbox','--renderer-process-limit=2']});
try {
 const page=await browser.newPage({viewport:{width:1440,height:1000}});page.on('pageerror',e=>errors.push(String(e)));
 await page.route('**/*',route=>new URL(route.request().url()).origin===new URL(base).origin?route.continue():route.abort());
 for(const language of ['en','de','pt','es','it','fr','zh-CN']) {
  await page.goto(`${base}/?ui_lang=${language}#/indexing/generations`);
  await page.locator('.generation-view .catalog tbody tr').first().waitFor();
  assert.ok(Number((await page.locator('.generation-view .active strong').innerText()).replace(/[^0-9]/g,''))>0);
  assert.equal(await page.locator('.bucket-bar i').count(),16);
  assert.ok(await page.locator('.release-line .passed').count()>=3);
  assert.match(await page.locator('.release-line > div').last().getAttribute('class'),/passed/);
  assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
  if(language==='en')await page.screenshot({path:join(output,'generations-desktop.png'),fullPage:true});
 }
 await page.setViewportSize({width:390,height:844});await page.goto(`${base}/?ui_lang=es#/indexing/generations`);
 await page.locator('.generation-view .catalog tbody tr').first().waitFor();assert.ok(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth+1));
 await page.screenshot({path:join(output,'generations-mobile.png'),fullPage:true});
 await page.goto(`${base}/?ui_lang=en#/indexing/overview`);await page.locator('.preview form input').fill('search');await page.locator('.preview form button').click();
 await page.locator('.preview article').first().waitFor({timeout:10000});
 assert.ok(await page.locator('.preview article').count()>0,'real selected-generation query should return documents');
 const indexing=await (await fetch(`${base}/api/indexing`)).json();const crawler=await (await fetch(`${base}/api/status`)).json();const generations=await (await fetch(`${base}/api/indexing/generations`)).json();
 assert.equal(indexing.meta.process_alive,true);assert.equal(crawler.meta.process_alive,true);assert.ok(generations.active);assert.deepEqual(errors,[]);
 const report={version:1,passed:true,checked_at:new Date().toISOString(),languages:7,browser_errors:errors,mobile_overflow:false,real_generation_query:true,crawler_pid:crawler.snapshot.pid,indexer_pid:indexing.snapshot.pid,documents:indexing.snapshot.totals.indexed_documents,active_generation:generations.active,api_read_only:true};
 await writeFile(join(root,'docs/indexador-i06-i08-dashboard-2026-10-14.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
}finally{await browser.close();}
