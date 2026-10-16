<script lang="ts">
  import { onMount } from 'svelte';
  import { getI18n } from './i18n';
  const {t,formatLocale}=getI18n();
  let {paused=false}: {paused?:boolean}=$props();
  type Progress = {url:string;crawl_state:string;search_state:string;priority?:boolean;reason?:string;index_reason?:string;not_before?:number;generation?:string};
  type Command = {id:string;urls:string[];state:string;requested_at:string;admitted?:number;not_admitted?:number;error?:string;progress?:Progress[]};
  let domains=$state(''),busy=$state(false),error=$state(''),readError=$state(''),message=$state(''),pendingId=$state('');
  let prioritize=$state(true),commands=$state<Command[]>([]);
  let prioritySites=$state<{name:string;urls:string[]}[]>([]);
  let manualReserve=$state(0);
  let refresh=async()=>{};
  function crawlLabel(row:Progress) {
    if(row.crawl_state==='pending')return row.priority?'Priority queued':'Queued';
    return ({fetching:'Fetching',captured:'Captured',terminal:'Crawl excluded',not_admitted:'Not admitted'} as Record<string,string>)[row.crawl_state]||'Unverified';
  }
  function searchLabel(row:Progress) {
    return ({published:'Published in search',excluded:'Index excluded',not_published:'Not yet published'} as Record<string,string>)[row.search_state]||'Unverified';
  }
  function progressRows(command:Command):Progress[] {
    return command.progress??command.urls.map(url=>({url,crawl_state:'unknown',search_state:'unknown'}));
  }
  onMount(()=>{
    let stopped=false,reading=false;const controller=new AbortController();
    refresh=async()=>{
      if(stopped||reading||paused)return;
      reading=true;
      try {
        const r=await fetch('/api/crawler/seeds',{signal:AbortSignal.any([controller.signal,AbortSignal.timeout(3000)])});
        if(!r.ok)throw Error('Manual admission unavailable');
        const value=await r.json();
        if(!stopped){
          commands=value.commands;prioritySites=value.priority_sites??[];manualReserve=value.coverage?.manual_reserve_per_site??0;readError='';
          if(pendingId&&commands.some(command=>command.id===pendingId&&command.state!=='queued')){message='';pendingId='';}
        }
      }catch{if(!stopped)readError='Manual admission unavailable';}
      finally{reading=false;}
    };
    void refresh();const timer=setInterval(()=>void refresh(),2000);
    return()=>{stopped=true;clearInterval(timer);controller.abort();};
  });
  async function submit(selected?:string[],asPriority=prioritize){
    busy=true;error='';message='';
    try{
      const urls=selected??domains.split(/\r?\n/).map(v=>v.trim()).filter(Boolean);
      if(!urls.length||urls.length>16)throw Error('Submit 1 to 16 domains or URLs');
      const r=await fetch('/api/crawler/seeds',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id:crypto.randomUUID(),urls,priority:asPriority}),signal:AbortSignal.timeout(5000)});
      const result=await r.json();if(!r.ok)throw Error(result.error);
      pendingId=result.id;message='Admission queued; waiting for crawler confirmation.';
      if(!selected)domains='';await refresh();
    }catch(reason){error=reason instanceof Error?reason.message:String(reason);}
    finally{busy=false;}
  }
</script>

<section class="panel admission">
  <header><div><p>{$t('Local administrator control')}</p><h2>{$t('Manual domains')}</h2></div><span>{$t('No crawler restart required')}</span></header>
  <form onsubmit={event=>{event.preventDefault();void submit();}}>
    <label for="manual-domains">{$t('Domains or URLs, one per line')}</label>
    <textarea id="manual-domains" bind:value={domains} rows="4" maxlength="16000" required spellcheck="false"></textarea>
    <div class="submit-row">
      <label class="priority"><input type="checkbox" bind:checked={prioritize}/>{$t('Prioritize these URLs')}</label>
      <button class="button primary" disabled={busy||!domains.trim()}>{$t(busy?'Submitting…':'Add to crawl queue')}</button>
    </div>
  </form>
  <p class="policy">{$t('Priority respects robots, remote delays and quotas. Discovery retains regular turns.')}</p>
  <p class="policy">{$t('Up to 16 per request. Bare domains use HTTPS.')}</p>
  {#if manualReserve}<p class="policy">{$t('Manual reserve: {p0} additional URLs per site and round.',{p0:manualReserve})}</p>{/if}
  {#if error||readError}<p role="alert">{$t(error||readError)}</p>{/if}
  {#if message}<p role="status">{$t(message)}</p>{/if}
</section>

{#if prioritySites.length}
  <section class="panel cohort">
    <header><h2>{$t('Coverage priorities')}</h2><button class="button" disabled={busy} onclick={()=>void submit(prioritySites.flatMap(site=>site.urls),true)}>{$t('Queue priority sites')}</button></header>
    <div class="site-list">{#each prioritySites as site}<span>{site.name}</span>{/each}</div>
    <p class="policy">{$t('Operator-selected sites; not a measured traffic ranking.')}</p>
  </section>
{/if}

<section class="panel history">
  <header><h2>{$t('Recent manual requests')}</h2><span>{commands.length} / 50</span></header>
  {#if !commands.length}<p>{$t('No manual domains have been submitted.')}</p>{/if}
  {#each commands as command}
    <article>
      <div class="command-head"><time>{new Date(command.requested_at).toLocaleString($formatLocale)}</time><span>{$t(command.state==='completed'?'Admission processed':command.state)}</span></div>
      <ul>{#each progressRows(command) as row}
        <li>
          <div class="url-row">
            <a href={row.url} target="_blank" rel="noreferrer">{row.url}</a>
            <span class="stage" data-state={row.crawl_state}>{$t(crawlLabel(row))}</span>
            <span class="stage" data-state={row.search_state} title={row.generation}>{$t(searchLabel(row))}</span>
          </div>
          {#if row.index_reason||(row.reason&&['pending','terminal','not_admitted'].includes(row.crawl_state))}<p class="reason">{row.index_reason||row.reason}</p>{/if}
          {#if row.crawl_state==='pending'&&row.not_before&&row.not_before>Date.now()}<p class="reason">{$t('Next eligible')}: {new Date(row.not_before).toLocaleString($formatLocale)}</p>{/if}
        </li>
      {/each}</ul>
      {#if command.state==='completed'}<p class="receipt"><b>{command.admitted??0}</b> {$t('Admitted to frontier')} · <b>{command.not_admitted??0}</b> {$t('Already known or excluded by policy/budget')}</p>
      {:else if command.state==='queued'}<p>{$t('Waiting for crawler confirmation.')}</p>{/if}
      {#if command.error}<p role="alert">{command.error}</p>{/if}
      <small>{command.id}</small>
    </article>
  {/each}
</section>

<style>
  .panel{padding:24px;margin-bottom:24px}header{display:flex;justify-content:space-between;align-items:center;gap:24px;margin-bottom:24px}h2{font-size:18px;margin:0}header span,header p,small,time{color:#61716a;font-size:12px}header p{margin:0 0 8px}form>label{display:block;font-size:14px;margin-bottom:12px}textarea{display:block;width:100%;max-width:760px;box-sizing:border-box;border:1px solid #cbd5d0;border-radius:6px;padding:16px;font:14px/1.6 system-ui,sans-serif;resize:vertical;color:#25372d;background:white}textarea:focus-visible{outline:3px solid #73aff4;outline-offset:2px}.submit-row{display:flex;justify-content:space-between;align-items:center;gap:24px;max-width:760px;margin-top:16px}.priority{display:flex;align-items:center;gap:8px;font-size:13px}.policy,.history p{font-size:12px;line-height:1.6;color:#61716a}.policy{max-width:760px;margin-top:16px}.history article{border-top:1px solid #e0e7e2;padding:24px 0}.command-head{display:flex;justify-content:space-between;gap:16px;font-size:12px;color:#61716a}ul{list-style:none;padding:0;margin:16px 0;font-size:14px;line-height:1.7}li{padding:8px 0}.url-row{display:grid;grid-template-columns:minmax(0,1fr) 150px 150px;align-items:baseline;gap:16px}.url-row a{color:#175b9c;overflow-wrap:anywhere;text-decoration:none}.url-row a:hover{text-decoration:underline}.stage{font-size:12px;color:#61716a}.stage[data-state="published"],.stage[data-state="captured"]{color:#326d50;font-weight:600}.stage[data-state="pending"]{color:#8a6308}.stage[data-state="excluded"],.stage[data-state="terminal"]{color:#9c3b30}.history .reason{margin:4px 0;color:#8a6308}.history .receipt{margin:16px 0 8px}.history small{font-size:10px}.site-list{display:flex;flex-wrap:wrap;gap:12px}.site-list span{padding:8px 12px;background:#f1f5f2;border-radius:4px;font-size:13px}@media(max-width:750px){.panel{padding:16px}header,.submit-row{align-items:flex-start;flex-direction:column;gap:12px}.url-row{grid-template-columns:1fr 1fr;gap:8px}.url-row a{grid-column:1/-1}}
</style>
