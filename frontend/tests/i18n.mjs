import assert from 'node:assert/strict';
import { readFile, writeFile } from 'node:fs/promises';
import { chromium } from '../node_modules/playwright/index.mjs';
const base=process.env.ASTIANGO_SITE_URL||'http://127.0.0.1:8000';
const codes=['en','de','pt','es','it','fr','zh-CN'];
const catalogs=Object.fromEntries(await Promise.all(codes.map(async code=>[code,JSON.parse(await readFile(new URL(`../../packages/i18n/messages/${code}.json`,import.meta.url),'utf8'))])));
const routes=['/','/settings','/settings/privacy','/settings/sites','/settings/optics','/about','/webmasters','/privacy-and-happy-lawyers','/explore'];
const result={_type:'websites',hasMoreResults:true,numHits:{_type:'exact',value:1234},searchDurationMs:1234,
 webpages:[{domain:'example.test',site:'https://example.test/',prettyUrl:'example.test',url:'https://example.test/?sig=a%2Fb&x=2',title:'Search',likelyHasAds:false,likelyHasPaywall:false,snippet:{text:{fragments:[{kind:'normal',text:'Untranslated source content: Search'}]}}}]};
const browser=await chromium.launch({headless:true,executablePath:process.env.CHROMIUM_PATH||'/usr/bin/chromium',args:['--no-sandbox']});
try{
 const context=await browser.newContext({viewport:{width:1440,height:1000}});const page=await context.newPage(),errors=[],external=[],queries=[];page.on('pageerror',e=>errors.push(String(e)));
 await page.route('**/*',async route=>{
  const url=new URL(route.request().url());
  if(url.pathname.startsWith('/improvement/')){await route.fulfill({body:'fixture-query-id',headers:{'access-control-allow-origin':'*'}});return;}
  if(url.pathname.includes('/beta/api/')){
   let data=null;
   if(url.pathname.endsWith('/autosuggest')||url.pathname.endsWith('/similar'))data=[];
   if(url.pathname.endsWith('/search')){data=result;queries.push(JSON.parse(route.request().postData()||'{}'));}
   await route.fulfill({json:data,headers:{'access-control-allow-origin':'*'}});return;
  }
  if(url.origin===base||url.protocol==='data:')return route.continue();external.push(url.href);return route.abort();
 });
 const wait=async check=>{for(let i=0;i<120;i++){if(await check())return;await page.waitForTimeout(50);}throw Error('UI did not settle');};
 for(const code of codes){
  for(const path of routes){
   const response=await page.goto(`${base}${path}?ui_lang=${code}`);assert.equal(response.status(),200,`${code} ${path}`);
   await wait(async()=>await page.locator('html').getAttribute('lang')===code);
   await wait(async()=>await page.locator('.language-select select').inputValue()===code);
   if(path==='/')await page.getByText(catalogs[code]['Search the open web on your terms.'],{exact:true}).waitFor();
   if(path==='/privacy-and-happy-lawyers') assert.equal(new URL(await page.locator('article a[href*="/settings/privacy"]').getAttribute('href'),base).searchParams.get('ui_lang'),code);
   if(path==='/settings'){
    await page.getByRole('heading',{name:catalogs[code].Preferences,exact:true}).waitFor();
    assert.equal(await page.locator('#theme option').first().textContent(),catalogs[code].Auto);
    const requestType=page.getByRole('combobox',{name:catalogs[code]['Select request type'],exact:true});
    assert.deepEqual(await requestType.locator('option').allTextContents(),['GET','POST']);
   }
   assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,`${code} ${path}`);
   const html=await(await fetch(`${base}${path}?ui_lang=${code}`)).text();assert.match(html,new RegExp(`<html lang="${code}"`));
  }
  await page.goto(`${base}/search?q=immutable&ui_lang=${code}`);await page.locator('.qwant-result .result-title').first().waitFor();
  assert.equal((await page.locator('.qwant-result .result-title').first().textContent()).trim(),'Search');
  assert.match(await page.locator('.qwant-result').first().textContent(),/Untranslated source content: Search/);
  assert.equal(await page.locator('.qwant-result .result-title').first().getAttribute('href'),result.webpages[0].url);
  assert.equal(new URL(await page.getByRole('link',{name:catalogs[code]['Next page'],exact:true}).getAttribute('href'),base).searchParams.get('ui_lang'),code);
  assert.ok(queries.at(-1).query==='immutable');assert.ok(!Object.hasOwn(queries.at(-1),'ui_lang'));
 }
 await page.goto(base);await page.locator('.language-select select').selectOption('fr');await wait(async()=>new URL(page.url()).searchParams.get('ui_lang')==='fr');
 await page.getByText(catalogs.fr['Search the open web on your terms.'],{exact:true}).waitFor();
 await page.getByRole('link',{name:catalogs.fr.Settings,exact:true}).first().click();await page.getByRole('heading',{name:catalogs.fr.Preferences,exact:true}).waitFor();
 await page.reload();assert.equal(await page.locator('html').getAttribute('lang'),'fr');
 await page.goto(base+'/about?ui_lang=en');await page.locator('.language-select select').selectOption('zh-CN');await wait(async()=>new URL(page.url()).searchParams.get('ui_lang')==='zh-CN');await page.getByRole('heading',{name:'关于我们',exact:true}).waitFor();
 await page.setViewportSize({width:390,height:844});
 for(const code of codes){await page.goto(base+'/?ui_lang='+code);await wait(async()=>await page.locator('html').getAttribute('lang')===code);assert.equal(await page.evaluate(()=>document.documentElement.scrollWidth>innerWidth),false,`mobile ${code}`);}
 const noJs=await browser.newContext({javaScriptEnabled:false});const noJsPage=await noJs.newPage();
 await noJsPage.goto(base+'/?ui_lang=de');assert.equal(await noJsPage.locator('html').getAttribute('lang'),'de');
 await noJsPage.locator('.language-select select').selectOption('fr');await noJsPage.getByRole('button',{name:catalogs.de['Apply language'],exact:true}).click();
 assert.equal(await noJsPage.locator('html').getAttribute('lang'),'fr');await noJs.close();
 assert.deepEqual(errors,[]);assert.deepEqual(external,[]);
 // Parallel SSR requests must never share language preferences.
 const rendered=await Promise.all(codes.map(async code=>({code,html:await(await fetch(base+'/settings?ui_lang='+code)).text()})));
 for(const {code,html}of rendered){assert.match(html,new RegExp(`<html lang="${code}"`));assert.ok(html.includes(catalogs[code].Preferences));}
 const data={passed:true,languages:codes,routes:routes.length+1,ssrIsolated:true,languageSelectionWithoutJavaScript:true,resultContentPreserved:true,apiQueriesPreserved:true,selectionPersisted:true,mobile:true,pageErrors:errors};
 await writeFile(new URL('../../data/dashboard-i18n/site-verification.json',import.meta.url),JSON.stringify(data,null,2)+'\n');console.log(JSON.stringify(data));
}finally{await browser.close()}
