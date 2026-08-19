<svelte:options accessors />

<script lang="ts">
  import { type DisplayedWebpage } from '$lib/api';
  import type { SearchResults } from '$lib/search';
  import { onMount } from 'svelte';

  import Modal from './Modal.svelte';
  import Result from './Result.svelte';
  import Widget from './Widget.svelte';
  import Discussions from './Discussions.svelte';
  import Sidebar from './Sidebar.svelte';

  import ChevronLeft from '~icons/heroicons/chevron-left-20-solid';
  import ChevronRight from '~icons/heroicons/chevron-right-20-solid';
  import { flip } from 'svelte/animate';
  import SpellCorrection from './SpellCorrection.svelte';

  export let results: SearchResults;
  export let query: string;
  export let nextPageSearchParams: URLSearchParams | null;
  export let prevPageSearchParams: URLSearchParams | null;
  export let currentPage: number;
  export let spellCorrectElem: SpellCorrection | undefined;
  export let resultElems: Result[];

  let modal: { top: number; left: number; site: DisplayedWebpage } | undefined;

  onMount(() => {
    const listener = () => {
      modal = void 0;
    };
    document.addEventListener('click', listener);
    return () => document.removeEventListener('click', listener);
  });

  const openSearchModal =
    (site: DisplayedWebpage) =>
    ({ detail: button }: CustomEvent<HTMLButtonElement>) => {
      const rect = button.getBoundingClientRect();

      if (modal?.site == site) {
        modal = void 0;
        return;
      }

      // NOTE: The point calculated is the middle of the right edge of the clicked
      // element, like so:
      //     +---+
      //     |   x <--
      //     +---+
      modal = {
        top: window.scrollY + rect.top + rect.height / 2,
        left: window.scrollX + rect.right,
        site,
      };
    };

  $: {
    results;
    modal = void 0;
  }
</script>

{#if modal}
  <Modal
    {modal}
    on:close={() => {
      modal = void 0;
    }}
  />
{/if}

{#if results._type == 'websites'}
  <h1 class="sr-only">Search Results</h1>
  <div class="qwant-serp">
    {#if results.spellCorrection}
      <SpellCorrection spellCorrection={results.spellCorrection} bind:this={spellCorrectElem} />
    {/if}

    {#if results.widget}
      <Widget widget={results.widget} />
    {/if}

    {#if results.webpages}
      <div class="results-list">
        {#each results.webpages as webpage, resultIndex (`${query}-${resultIndex}-${webpage.url}`)}
          <div class="result-row" animate:flip={{ duration: 150 }} aria-expanded={modal?.site == webpage}>
            <Result
              bind:this={resultElems[resultIndex]}
              {webpage}
              {resultIndex}
              on:modal={openSearchModal(webpage)}
            />
          </div>
        {/each}
        {#if results.discussions}
          <Discussions discussions={results.discussions} />
        {/if}
      </div>
    {/if}

    <div class="pagination-wrap">
      <div class="pagination">
        {#if prevPageSearchParams}
          <a href="/search?{prevPageSearchParams}" aria-label="Previous page">
            <ChevronLeft
              class="text-xl text-primary hover:text-primary-focus"
              aria-label="Chevron left"
            />
          </a>
        {:else}
          <ChevronLeft class="text-xl text-neutral" aria-label="Chevron left" />
        {/if}
        <div>Page {currentPage}</div>
        {#if nextPageSearchParams}
          <a href="/search?{nextPageSearchParams}" aria-label="Next page">
            <ChevronRight
              class="text-xl text-primary hover:text-primary-focus"
              aria-label="Chevron right"
            />
          </a>
        {:else}
          <ChevronRight class="text-xl text-neutral" aria-label="Chevron right" />
        {/if}
      </div>
    </div>
  </div>

  {#if results.sidebar}
    <aside class="qwant-sidebar">
      <Sidebar sidebar={results.sidebar} />
    </aside>
  {/if}
{/if}

<style>
  .qwant-serp { grid-column: 1; display: flex; min-width: 0; max-width: 760px; flex-direction: column; gap: var(--space-8); } .results-list { display: grid; gap: var(--space-8); width: 100%; } .result-row { min-width: 0; } .pagination-wrap { display: flex; justify-content: center; padding-top: var(--space-4); } .pagination { display: grid; grid-template-columns: repeat(3, auto); align-items: center; gap: var(--space-3); color: var(--color-ink-soft); font: 600 13px var(--font-body); } .pagination :global(a) { color: var(--color-accent-strong); } .qwant-sidebar { grid-column: 2; grid-row: 2; align-self: start; min-width: 0; border: var(--rule) solid var(--color-rule); border-radius: var(--radius-lg); background: var(--color-paper-raised); box-shadow: var(--shadow-card); padding: var(--space-6); }
  @media (max-width: 900px) { .qwant-sidebar { grid-column: 1; grid-row: auto; } }
</style>
