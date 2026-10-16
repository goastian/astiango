<script lang="ts">
  import { afterNavigate } from '$app/navigation';
  import { navigating, page } from '$app/stores';
  import { tick } from 'svelte';
  import { resultsInNewTab, useKeyboardShortcuts } from '$lib/stores';
  import { localizedHref } from '../../../../packages/i18n';
  import { getI18n } from '$lib/i18n';
  import Searchbar from '$lib/components/Searchbar.svelte';
  import type { PageData } from './$types';
  const { t, locale, formatLocale }=getI18n();
  export let data: PageData;
  $: result=data.result;
  $: currentPage=result?.page ?? 1;
  $: message=data.failure?.code==='unsupported_option' ? 'This search option is not available.' : data.failure?.status===400 ? 'Check your query and search options.' : data.failure?.status===429 ? 'Search is busy. Please try again shortly.' : 'Search is temporarily unavailable. Please try again.';
  function pageHref(value:number){const params=new URLSearchParams($page.url.searchParams);params.set('p',String(value));return localizedHref(`/search?${params}`,$locale);}
  let sent='';
  let searchbar: Searchbar;
  function keyboard(event: KeyboardEvent) {
    const target=event.target as HTMLElement;
    if(!$useKeyboardShortcuts || event.ctrlKey || event.metaKey || event.altKey || event.isComposing || target?.closest('input,textarea,select,[contenteditable="true"]'))return;
    if(event.key==='/'){event.preventDefault();searchbar?.getInputElem()?.focus();}
    if(['j','k'].includes(event.key)){
      const links=[...document.querySelectorAll<HTMLAnchorElement>('.web-result h2 a')];
      const index=links.indexOf(document.activeElement as HTMLAnchorElement);
      const next=Math.max(0,Math.min(links.length-1,index+(event.key==='j'?1:-1)));
      if(links[next]){event.preventDefault();links[next].focus();}
    }
  }
  afterNavigate(async()=>{
    await tick();
    if(!data.result || sent===data.result.id)return;
    const id=data.result.id;sent=id;
    requestAnimationFrame(()=>{
      const resources=performance.getEntriesByType('resource').filter(e=>e.name.includes('/search/__data.json')) as PerformanceResourceTiming[];
      const navigation=performance.getEntriesByType('navigation')[0] as PerformanceNavigationTiming|undefined;
      const end=resources.at(-1)?.responseEnd ?? navigation?.responseEnd;
      if(end!==undefined)void fetch('/api/web/experience',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({id,domReadyMs:Math.max(0,performance.now()-end)}),keepalive:true}).catch(()=>{});
    });
  });
</script>
<svelte:window on:keydown={keyboard} />
<div class="results-shell">
  <header class="results-head">
    <Searchbar query={data.query} bind:this={searchbar} />
    <div class="search-tools">
      <nav aria-label={$t('Search result types')}><a aria-current="page" href={localizedHref(`/search?q=${encodeURIComponent(data.query)}`,$locale)}>{$t('Web')}</a></nav>
      <form method="GET" action="/search" class="safe-filter">
        <input type="hidden" name="q" value={data.query}/><input type="hidden" name="ui_lang" value={$locale}/>
        <label for="safe-search">{$t('SafeSearch')}</label>
        <select id="safe-search" name="ss" value={$page.url.searchParams.get('ss')??'false'}><option value="false">{$t('Off')}</option><option value="true">{$t('On')}</option></select>
        <button type="submit">{$t('Apply')}</button>
      </form>
    </div>
  </header>
  <section class="results-body" aria-labelledby="results-title" aria-busy={!!$navigating} data-request-id={result?.id} data-generation={result?.generation}>
    <h1 id="results-title" class="sr-only">{$t('Search Results')}: {data.query}</h1>
    {#if $navigating}<p class="search-status" role="status">{$t('Searching…')}</p>{/if}
    {#if data.failure}
      <div class="search-state" role="alert"><h2>{$t('Search could not be completed')}</h2><p>{$t(message)}</p><a class="retry" href={$page.url.href} data-sveltekit-reload>{$t('Try again')}</a></div>
    {:else if result}
      <p class="result-count" role="status">
        {#if result.total.kind!=='unknown'}{result.total.kind==='approximate'?'≈ ':''}{(result.total.value??0).toLocaleString($formatLocale)} {$t('results')}{:else}{$t('Search Results')}{/if}
        <span aria-hidden="true"> · </span>{(result.durationMs/1000).toLocaleString($formatLocale,{maximumFractionDigits:2})} s
      </p>
      {#if result.navigationHost}<p class="result-count">{$t('Indexed pages from {p0}',{p0:result.navigationHost})}</p>{/if}
      {#if result.navigationFrom}<p class="result-count">{$t('Captured redirect: {p0} → {p1}',{p0:result.navigationFrom,p1:result.navigationHost??''})}</p>{/if}
      {#if result.results.length}
        <div class="results-list">
          {#each result.results as item,index (`${result.id}-${index}`)}
            <article class="web-result" data-result-url={item.url}>
              <div class="source"><span class="source-mark" aria-hidden="true">{item.host[0]?.toUpperCase()}</span><span class="source-host">{item.host}</span></div>
              <h2><a href={item.url} target={$resultsInNewTab ? '_blank' : undefined} rel="noreferrer">{item.title}</a></h2>
              {#if item.fragments.some(f=>f.text)}<p class="snippet">{#each item.fragments as fragment}{#if fragment.highlighted}<b>{fragment.text}</b>{:else}{fragment.text}{/if}{/each}</p>{/if}
            </article>
          {/each}
        </div>
      {:else}
        <div class="search-state"><h2>{$t('No results')}</h2><p>{$t(result.navigationHost ? 'No matching indexed pages were found for this domain.' : 'Try different keywords or a broader query.')}</p></div>
      {/if}
      <nav class="pagination" aria-label={$t('Pagination')}>
        {#if currentPage>1}<a href={pageHref(currentPage-1)} rel="prev">← {$t('Previous page')}</a>{/if}
        <span aria-current="page">{$t('Page {p0}',{p0:currentPage})}</span>
        {#if result.hasMore}<a href={pageHref(currentPage+1)} rel="next">{$t('Next page')} →</a>{:else if result.hasMore===null}<span>{$t('More results could not be confirmed.')}</span>{/if}
      </nav>
    {/if}
  </section>
</div>
<style>
  .results-shell{width:min(100%,1240px);margin:0 auto;padding:32px 32px 64px;font-family:var(--font-body)}.results-head,.results-body{width:min(100%,720px)}.search-tools{display:flex;align-items:center;justify-content:space-between;gap:16px;border-bottom:1px solid var(--color-rule);margin-top:16px}.search-tools nav a{display:block;padding:16px 8px 12px;border-bottom:2px solid var(--color-accent);color:var(--color-accent-strong);text-decoration:none;font-weight:600;font-size:14px}.safe-filter{display:flex;gap:8px;align-items:center;font-size:12px;color:var(--color-ink-soft)}.safe-filter select{padding:8px 24px 8px 8px;font-size:12px;border:1px solid var(--color-rule);border-radius:8px;background-color:var(--color-paper-raised);color:var(--color-ink)}.safe-filter button{min-height:44px;padding:8px;border:0;border-radius:8px;background:var(--color-paper-soft);color:var(--color-ink);cursor:pointer}.result-count,.search-status{margin:20px 0 24px;font-size:12px;color:var(--color-muted)}.results-list{display:grid;gap:32px}.web-result{min-width:0}.source{display:flex;align-items:center;gap:8px;margin-bottom:8px;color:var(--color-ink-soft);font-size:13px}.source-mark{display:grid;place-items:center;width:24px;height:24px;border:1px solid var(--color-rule);border-radius:8px;color:var(--color-accent-strong);background:var(--color-paper-soft);font-size:12px}.source-host{overflow-wrap:anywhere}h2{margin:0;font:500 20px/1.35 var(--font-body);letter-spacing:-.02em;overflow-wrap:anywhere}h2 a{color:var(--color-link);text-decoration:none}h2 a:visited{color:var(--color-link-visited)}h2 a:hover{text-decoration:underline}.snippet{margin:8px 0 0;color:var(--color-ink-soft);font:400 16px/1.65 var(--font-body);overflow-wrap:anywhere}.snippet b{font-weight:650;color:var(--color-ink)}.pagination{display:flex;align-items:center;gap:24px;flex-wrap:wrap;margin-top:48px;font-size:14px}.pagination a,.retry{display:inline-flex;align-items:center;min-height:44px;color:var(--color-accent-strong);font-weight:600;text-decoration:none}.pagination a:hover,.retry:hover{text-decoration:underline}.pagination span{color:var(--color-muted)}.search-state{padding:32px 0}.search-state p{color:var(--color-ink-soft);font-size:15px;line-height:1.6}a:focus-visible,button:focus-visible,select:focus-visible{outline:3px solid var(--color-focus);outline-offset:4px}@media(max-width:640px){.results-shell{padding:24px 16px 48px}.safe-filter{gap:4px}.search-tools{gap:8px}.results-list{gap:32px}h2{font-size:20px}.snippet{font-size:14px}}
</style>
