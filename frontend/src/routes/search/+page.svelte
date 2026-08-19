<script lang="ts">
  import OpticSelector from '$lib/components/OpticSelector.svelte';
  import Searchbar from '$lib/components/Searchbar.svelte';
  import type { PageData } from './$types';
  import RegionSelect from '$lib/components/RegionSelect.svelte';
  import {
    searchQueryStore,
    showRankingSignals,
    useKeyboardShortcuts,
    hostRankingsStore,
  } from '$lib/stores';
  import { page } from '$app/stores';
  import { updateQueryId } from '$lib/improvements';
  import { browser } from '$app/environment';
  import Serp from './Serp.svelte';
  import Result from './Result.svelte';
  import { search, type SearchParams } from '$lib/search';
  import { Keybind, searchCb, type Refs } from '$lib/keybind';
  import SpellCorrection from './SpellCorrection.svelte';
  import type { Count } from '$lib/api';
  import { match } from 'ts-pattern';
  import { rankingsToRanked, type RankedSites } from '$lib/rankings';
  import { derived } from 'svelte/store';

  export let data: PageData;
  $: results = data.results;
  $: query = data.params.query;

  let prevPageSearchParams: URLSearchParams | null = null;
  let nextPageSearchParams: URLSearchParams | null = null;

  $: {
    if (data.params.currentPage > 1) {
      const newParams = new URLSearchParams($page.url.searchParams);
      newParams.set('p', (data.params.currentPage - 1).toString());
      prevPageSearchParams = newParams;
    } else {
      prevPageSearchParams = null;
    }

    if (results && results._type == 'websites' && results.hasMoreResults) {
      const newParams = new URLSearchParams($page.url.searchParams);
      newParams.set('p', (data.params.currentPage + 1).toString());
      nextPageSearchParams = newParams;
    } else {
      nextPageSearchParams = null;
    }
  }

  let serp: Serp | undefined = undefined;

  const hostRankings = derived(hostRankingsStore, (rankings) => {
    return rankingsToRanked(rankings);
  });

  const clientSearch = async (dataParams: SearchParams, hostRankings: RankedSites) => {
    if (!browser) return;

    const params = {
      ...dataParams,
      showRankingSignals: $showRankingSignals,
      hostRankings,
    };

    const res = await search(params, { fetch: fetch });

    if (res._type == 'bang') {
      window.location.replace(res.redirectTo);
      return null;
    }

    results = res;

    return res;
  };

  // NOTE: save the search query to be used in the navbar
  $: searchQueryStore?.set($page.url.search);
  let paramsForRedirect = new URLSearchParams($page.url.search);
  let serverSearch = paramsForRedirect.get('ssr') === 'true';
  paramsForRedirect.set('ssr', 'true');
  let encodedQueryForRedirect = paramsForRedirect.toString();

  $: {
    if (browser && results && results._type == 'websites')
      updateQueryId({ query, webpages: results.webpages });
  }

  let resultElems: Result[] = [];
  let spellCorrectElem: SpellCorrection | undefined = undefined;
  let searchbarElem: Searchbar | undefined = undefined;

  let context: Refs;
  $: context = {
    results: serp?.resultElems,
    searchbar: searchbarElem,
    spellCorrection: serp?.spellCorrectElem,
  };

  let keybind = new Keybind([
    { key: 'j', callback: searchCb.focusNextResult },
    { key: 'ArrowDown', callback: searchCb.focusNextResult },
    { key: 'k', callback: searchCb.focusPrevResult },
    { key: 'ArrowUp', callback: searchCb.focusPrevResult },
    { key: 'h', callback: searchCb.selectSearchBar },
    { key: '/', callback: searchCb.selectSearchBar },
    { key: 'v', callback: searchCb.openResultInNewTab },
    { key: "'", callback: searchCb.openResultInNewTab },
    { key: 't', callback: searchCb.scrollToTop },
    { key: 'd', callback: searchCb.domainSearch },
    { key: 'l', callback: searchCb.openResult },
    { key: 'o', callback: searchCb.openResult },
    { key: 'm', callback: searchCb.focusMainResult },
    { key: 's', callback: searchCb.openSpellCorrection },
    { key: 'Escape', callback: searchCb.clearFocus },
  ]);

  const onKeyDown = (event: KeyboardEvent) => {
    if (event.target != searchbarElem?.getInputElem()) {
      keybind.onKeyDown(event, $useKeyboardShortcuts, context);
    }
  };

  const prettyprintCount = (count: Count): string => {
    return match(count)
      .with({ _type: 'exact' }, () => {
        return count.value.toLocaleString();
      })
      .with({ _type: 'approximate' }, () => {
        return `~${count.value.toLocaleString()}`;
      })
      .exhaustive();
  };

  $: clientResults = clientSearch(data.params, $hostRankings);
</script>

<svelte:window on:keydown={onKeyDown} />

{#if !serverSearch}
  <noscript>
    <meta http-equiv="refresh" content="0;url=/search?{encodedQueryForRedirect}" />
    <div>
      You are being redirected to <a href="/search?{encodedQueryForRedirect}" class="underline"
        >a page that doesn't require javascript.</a
      >
    </div>
  </noscript>
{/if}

<div class="qwant-results-shell">
  <div class="qwant-results-head">
    <div class="qwant-search-wrap">
      <Searchbar {query} bind:this={searchbarElem} />
    </div>
    <nav class="result-tabs" aria-label="Search result types">
      <a class="active" href="/search?q={encodeURIComponent(query)}">All</a>
      <a href="/search?q={encodeURIComponent(query)}&type=images">Images</a>
      <a href="/search?q={encodeURIComponent(query)}&type=videos">Videos</a>
      <a href="/search?q={encodeURIComponent(query)}&type=news">News</a>
      <a href="/search?q={encodeURIComponent(query)}&type=discussions">Discussions</a>
    </nav>
    <div class="result-tools">
      <div class="result-count">
          {#if results}
            Found <strong>{prettyprintCount(results.numHits)}</strong> results in
            <strong>{((results.searchDurationMs ?? 0) / 1000).toFixed(2)}s</strong>
          {/if}
      </div>
      <div class="result-selects">
        <div>
          <OpticSelector searchOnChange={true} selected={data.params.optic} />
        </div>
        <div>
          <RegionSelect searchOnChange={true} selected={data.params.selectedRegion} />
        </div>
      </div>
    </div>
  </div>
  {#if results}
    <Serp
      {results}
      {query}
      {prevPageSearchParams}
      {nextPageSearchParams}
      currentPage={data.params.currentPage}
      {resultElems}
      {spellCorrectElem}
      bind:this={serp}
    />
  {:else}
    {#await clientResults then results}
      {#if results}
        <Serp
          {results}
          {query}
          {prevPageSearchParams}
          {nextPageSearchParams}
          currentPage={data.params.currentPage}
          {resultElems}
          {spellCorrectElem}
          bind:this={serp}
        />
      {/if}
    {:catch}
      <section class="preview-results" aria-label="Search result preview">
        <p class="preview-label">Search result preview</p>
        <article>
          <p class="preview-url">astian.org</p>
          <a href="https://astian.org">AstianGO — private, open search</a>
          <p>Search the web through an independent index, without behavioural profiling or a personal data marketplace.</p>
        </article>
        <article>
          <p class="preview-url">github.com / goastian / astiango</p>
          <a href="https://github.com/goastian/astiango">AstianGO source code</a>
          <p>Explore the open-source search engine, its crawler and the infrastructure that powers its own index.</p>
        </article>
        <article>
          <p class="preview-url">astian.org / midori-browser</p>
          <a href="https://astian.org/midori-browser/">Midori Browser</a>
          <p>A lightweight browser from Astian, designed as a companion for a calmer and more private web.</p>
        </article>
      </section>
      <aside class="preview-panel" aria-label="AstianGO overview">
        <span class="panel-kicker">About this result</span>
        <h2>AstianGO</h2>
        <p>A private, open and transparent search engine with an independent crawler and index.</p>
        <a href="/about">Learn about AstianGO →</a>
      </aside>
    {/await}
  {/if}
</div>

<style>
  .qwant-results-shell { display: grid; width: 100%; grid-template-columns: minmax(0, 760px) minmax(0, 360px); column-gap: clamp(var(--space-8), 7vw, var(--space-20)); row-gap: var(--space-6); padding: var(--space-6) max(var(--space-5), calc((100vw - 1240px) / 2)) var(--space-16); text-rendering: optimizeLegibility; }
  .qwant-results-head { grid-column: 1 / -1; display: grid; grid-template-columns: minmax(0, 760px) minmax(0, 360px); column-gap: clamp(var(--space-8), 7vw, var(--space-20)); row-gap: var(--space-3); } .qwant-search-wrap { grid-column: 1; } .result-tabs { grid-column: 1; display: flex; gap: var(--space-5); overflow-x: auto; padding: var(--space-2) var(--space-1) 0; } .result-tabs a { position: relative; color: var(--color-muted); font: 700 13px var(--font-body); text-decoration: none; white-space: nowrap; } .result-tabs a:hover, .result-tabs a.active { color: var(--color-accent-strong); } .result-tabs a.active::after { position: absolute; right: 0; bottom: calc(var(--space-2) * -1); left: 0; height: 2px; border-radius: var(--radius-pill); background: var(--color-accent); content: ''; }
  .result-tools { grid-column: 1; display: flex; align-items: center; justify-content: space-between; gap: var(--space-4); } .result-count { color: var(--color-muted); font: 500 12px var(--font-body); } .result-count strong { color: var(--color-ink-soft); font-weight: 700; } .result-selects { display: flex; align-items: center; gap: var(--space-2); }
  .preview-results { grid-column: 1; display: grid; gap: var(--space-8); max-width: 760px; } .preview-label { margin: 0 0 calc(var(--space-2) * -1); color: var(--color-muted); font: 600 12px var(--font-body); } .preview-results article { display: grid; gap: var(--space-2); } .preview-results p { margin: 0; color: var(--color-ink-soft); font: 400 14px/1.58 var(--font-body); } .preview-results .preview-url { color: var(--color-success); font: 600 12px var(--font-body); } .preview-results a { color: var(--color-link); font: 650 clamp(18px, 2vw, 21px)/1.25 var(--font-display); letter-spacing: -.025em; text-decoration: none; } .preview-results a:hover { text-decoration: underline; } .preview-panel { grid-column: 2; align-self: start; display: grid; gap: var(--space-3); border: var(--rule) solid var(--color-rule); border-radius: var(--radius-lg); background: var(--color-paper-raised); box-shadow: var(--shadow-card); padding: var(--space-6); } .preview-panel h2, .preview-panel p { margin: 0; } .preview-panel h2 { color: var(--color-ink); font: 650 25px/1.1 var(--font-display); letter-spacing: -.04em; } .preview-panel p { color: var(--color-ink-soft); font: 400 14px/1.55 var(--font-body); } .preview-panel a { color: var(--color-accent-strong); font: 700 13px var(--font-body); text-decoration: none; } .preview-panel a:hover { text-decoration: underline; } .panel-kicker { color: var(--color-muted); font: 700 11px var(--font-body); letter-spacing: .08em; text-transform: uppercase; }
  @media (max-width: 900px) { .qwant-results-shell { grid-template-columns: minmax(0, 1fr); } .qwant-results-head { grid-template-columns: minmax(0, 1fr); } .qwant-search-wrap, .result-tabs, .result-tools, .preview-results, .preview-panel { grid-column: 1; } } @media (max-width: 560px) { .qwant-results-shell { padding-inline: var(--space-4); } .result-tools { align-items: flex-start; flex-direction: column; } .result-selects { width: 100%; justify-content: space-between; } }
</style>
