<svelte:options accessors />

<script lang="ts">
  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import AdjustVertical from '~icons/heroicons/adjustments-vertical';
  import type { DisplayedWebpage } from '$lib/api';
  import { createEventDispatcher } from 'svelte';
  import { markPagesWithAdsStore, markPagesWithPaywallStore } from '$lib/stores';
  import TextSnippet from '$lib/components/TextSnippet.svelte';
  import StackOverflowSnippet from './StackOverflowSnippet.svelte';
  import ResultLink from './ResultLink.svelte';
  import { hostRankingsStore } from '$lib/stores';
  import type { Ranking } from '$lib/rankings';
  import HandThumbDown from '~icons/heroicons/hand-thumb-down-20-solid';
  import HandThumbUp from '~icons/heroicons/hand-thumb-up-20-solid';

  export let webpage: DisplayedWebpage;
  export let resultIndex: number;

  let ranking: Ranking | undefined = undefined;
  hostRankingsStore.subscribe((rankings) => {
    if (rankings) {
      ranking = rankings[webpage.site];
    }
  });

  let button: HTMLButtonElement;

  const dispatch = createEventDispatcher<{ modal: HTMLButtonElement }>();

  let mainDiv: HTMLElement | undefined = undefined;
  export const getMainDiv = () => mainDiv;

  let mainResultLink: ResultLink | undefined = undefined;
  export const getMainResultLink = () => mainResultLink;

  export const hasFocus = () => mainResultLink?.hasFocus();
  export const clearFocus = () => mainResultLink?.clearFocus();
</script>

<article class="qwant-result" bind:this={mainDiv}>
    <div class="result-topline">
      <div class="result-main">
        <span class="result-copy">
          <h3>
            <ResultLink
              _class="title result-title"
              title={webpage.title}
              href={webpage.url}
              {resultIndex}
              bind:this={mainResultLink}
            >
              {webpage.title}
            </ResultLink>
          </h3>
          <div class="result-url-row">
            <ResultLink
              _class="url result-url"
              href={webpage.url}
              {resultIndex}
            >
              {webpage.prettyUrl}
            </ResultLink>
          </div>
        </span>
      </div>
      <div class="result-actions">
        {#if ranking}
          <span class="ranking">
            <span>
              {#if ranking == 'liked'}
                <div title={$t("liked site")} aria-label={$t("you have liked this site")}>
                  <HandThumbUp class="w-3 text-success" />
                </div>
              {:else if ranking == 'disliked'}
                <div aria-label={$t("you have disliked this site")} title={$t("disliked site")}>
                  <HandThumbDown class="w-3 text-warning" />
                </div>
              {/if}
            </span>
          </span>
        {/if}
        <button
          class="result-menu"
          aria-label={$t("Open modal for result number: {p0}", { p0: resultIndex })}
          bind:this={button}
          on:click|stopPropagation={() => dispatch('modal', button)}
        >
          <AdjustVertical class="text-md" aria-label={$t("3 vertical bars")} />
        </button>
      </div>
    </div>
    <p class="snippet result-snippet [&>b]:font-bold">
      {#if webpage.richSnippet && webpage.richSnippet._type == 'stackOverflowQA'}
        <StackOverflowSnippet
          question={webpage.richSnippet.question}
          answers={webpage.richSnippet.answers}
        />
      {:else}
        <span class="line-clamp-4 md:line-clamp-3">
          <span class="inline">
            <span id="snippet-text" class="snippet-text">
              {#if webpage.likelyHasAds && $markPagesWithAdsStore && webpage.likelyHasPaywall && $markPagesWithPaywallStore}
                <span
                  class="rounded border border-primary p-0.5 text-center text-xs text-neutral"
                  title={$t("page likely has ads and paywall")}
                >
                  {$t("has ads + paywall")}
                </span>
              {:else if webpage.likelyHasAds && $markPagesWithAdsStore}
                <span
                  class="rounded border border-primary p-0.5 text-center text-xs text-neutral"
                  title={$t("page likely has ads")}
                >
                  {$t("has ads")}
                </span>
              {:else if webpage.likelyHasPaywall && $markPagesWithPaywallStore}
                <span
                  class="rounded border border-primary p-0.5 text-center text-xs text-neutral"
                  title={$t("page likely has paywall")}
                >
                  {$t("paywall")}
                </span>
              {/if}
              {#if webpage.snippet.date}
                <span class="text-neutral">
                  {webpage.snippet.date}
                </span> -
              {/if}
              <span>
                <TextSnippet snippet={webpage.snippet.text} />
              </span>
            </span>
          </span>
        </span>
      {/if}
    </p>
  </article>

<style>
  .qwant-result { display: flex; min-width: 0; flex-direction: column; gap: var(--space-2); padding: 0; font-family: var(--font-body); } .result-topline { display: flex; min-width: 0; align-items: flex-start; gap: var(--space-3); } .result-main { min-width: 0; flex: 1; } .result-copy { display: flex; flex-direction: column-reverse; gap: var(--space-1); } h3 { margin: 0; min-width: 0; } :global(.result-title) { display: block; max-width: 100%; overflow: hidden; color: var(--color-link); font: 650 clamp(18px, 2vw, 21px)/1.25 var(--font-display); letter-spacing: -.025em; text-decoration: none; text-overflow: ellipsis; white-space: nowrap; } :global(.result-title:visited) { color: var(--color-link-visited); } :global(.result-title:hover) { text-decoration: underline; } .result-url-row { display: flex; min-width: 0; align-items: center; } :global(.result-url) { display: block; max-width: 100%; overflow: hidden; color: var(--color-success); font: 600 12px/1.3 var(--font-body); text-decoration: none; text-overflow: ellipsis; white-space: nowrap; } .result-actions { display: flex; align-items: center; gap: var(--space-2); } .ranking { color: var(--color-muted); font-size: 12px; } .result-menu { display: grid; width: 28px; height: 28px; place-items: center; border: 0; border-radius: 50%; background: transparent; color: var(--color-muted); cursor: pointer; } .result-menu:hover { background: var(--color-paper-soft); color: var(--color-ink); } .result-menu:focus-visible { outline: 3px solid var(--color-focus); outline-offset: 2px; } .result-snippet { margin: 0; color: var(--color-ink-soft); font: 400 14px/1.58 var(--font-body); } .result-snippet :global(b) { color: var(--color-ink); } @media (max-width: 540px) { :global(.result-title) { white-space: normal; } }
</style>
