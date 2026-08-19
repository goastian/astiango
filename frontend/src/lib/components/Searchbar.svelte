<svelte:options accessors />

<script lang="ts">
  import MagnifyingGlass from '~icons/heroicons/magnifying-glass';
  import { api, type HighlightedFragment } from '$lib/api';
  import { safeSearchStore, postSearchStore } from '$lib/stores';
  import { browser } from '$app/environment';
  import { twJoin } from 'tailwind-merge';
  import { P, match } from 'ts-pattern';

  export let autofocus = false;
  export let query = '';

  let selected: 'none' | number = 'none';
  let suggestions: HighlightedFragment[][] = [];

  let cancelLastRequest: null | (() => void) = null;

  const suggestionText = (s: HighlightedFragment[]): string => s.map((x) => x.text).join('');

  const updateSuggestions = (query: string) => {
    cancelLastRequest?.();

    if (!query) {
      suggestions = [];
      return;
    }

    const { data, cancel } = api.autosuggest({ q: query });
    cancelLastRequest = cancel;
    data
      .then((res) => (suggestions = res.map((x) => x.highlighted)))
      .catch(() => (suggestions = []));
  };

  let didChangeInput = false;
  let lastRealQuery = query;

  $: if (didChangeInput) lastRealQuery = query;
  $: if (browser) updateSuggestions(lastRealQuery);

  const selectSuggestion = (s: HighlightedFragment[]) => (query = suggestionText(s));

  const moveSelection = (step: number) => {
    selected = match(selected)
      .returnType<'none' | number>()
      .with(P.string, () => (step > 0 ? 0 : suggestions.length - 1))
      .with(
        P.when((v) => !(0 <= v + step && v + step < suggestions.length)),
        () => 'none',
      )
      .otherwise((v) => (v + suggestions.length + step) % suggestions.length);
    query = typeof selected == 'number' ? suggestionText(suggestions[selected]) : lastRealQuery;
    didChangeInput = false;
  };

  const onKeydown = (ev: KeyboardEvent) => {
    match(ev.key)
      .with('ArrowUp', () => {
        ev.preventDefault();
        moveSelection(-1);
      })
      .with('ArrowDown', () => {
        ev.preventDefault();
        moveSelection(1);
      })
      .with('Enter', () => {
        hasFocus = false;
      })
      .otherwise(() => {
        didChangeInput = true;
      });
  };

  let suggestionsDiv: HTMLDivElement | undefined;
  let hasFocus = autofocus;

  let formElem: HTMLFormElement;
  let inputElem: HTMLInputElement;
  export const getInputElem = () => inputElem;
  export const getForm = () => formElem;
  export const select = () => inputElem.select();
  export const userQuery = () => lastRealQuery;
  export const search = (q: string) => {
    if (formElem && inputElem) {
      inputElem.value = q;
      formElem.submit();
    }
  };
</script>

<form
  action="/search"
  class="qwant-search-form"
  id="searchbar-form"
  method={$postSearchStore ? 'POST' : 'GET'}
  bind:this={formElem}
>
  <input type="hidden" value={$safeSearchStore ? 'true' : 'false'} name="ss" />

  <label
    for="searchbar"
    class={twJoin(
      'qwant-search-field',
      hasFocus && suggestions.length > 0 && 'has-suggestions',
    )}
    aria-autocomplete="list"
    aria-expanded={suggestions.length > 0 && hasFocus}
  >
    <MagnifyingGlass class="search-glyph" aria-label="Magnifying glass" />
    <!-- svelte-ignore a11y-autofocus -->
    <input
      id="searchbar"
      name="q"
      {autofocus}
      placeholder="Search the web privately"
      autocomplete="off"
      aria-expanded={suggestions.length > 0 && hasFocus}
      class="search-input"
      on:focus={() => {
        hasFocus = true;
      }}
      on:blur={(e) => {
        // NOTE: If we click an element inside the suggestions,
        // don't blur yet since the clicked element would disapper
        if (e.relatedTarget instanceof Node && suggestionsDiv?.contains(e.relatedTarget)) return;

        // @ts-expect-error requestIdleCallback is not supported in Safari
        // https://caniuse.com/requestidlecallback
        if (window.requestIdleCallback) {
          requestIdleCallback(() => (hasFocus = false));
        } else {
          setTimeout(() => (hasFocus = false), 0);
        }
      }}
      bind:value={query}
      on:keydown={onKeydown}
      bind:this={inputElem}
    />
    <button class="search-submit" type="submit" aria-label="Search">
      <MagnifyingGlass aria-hidden="true" />
    </button>

    {#if suggestions.length > 0}
      <div
        class="suggestion-divider"
      ></div>
      <div
        class={twJoin(
          'suggestion-divider',
          hasFocus ? 'is-visible' : 'is-hidden',
        )}
      ></div>
      <div
        class={twJoin(
          'suggestions-panel',
          hasFocus ? 'is-visible' : 'is-hidden',
        )}
        role="listbox"
        bind:this={suggestionsDiv}
      >
        <ul class="w-full">
          {#each suggestions as s, index}
            <li>
              <button
                class={twJoin(
                  'suggestion-row',
                  selected == index && 'is-selected',
                )}
                on:click={() => {
                  selectSuggestion(s);
                  hasFocus = false;
                }}
                type="submit"
              >
                <MagnifyingGlass class="suggestion-icon" aria-label="Magnifying glass" />
                <span>
                  {#each s as fragment}
                    {#if fragment.kind == 'highlighted'}
                      <span class="font-medium">{fragment.text}</span>
                    {:else}
                      {fragment.text}
                    {/if}
                  {/each}
                </span></button
              >
            </li>
          {/each}
        </ul>
      </div>
    {/if}
  </label>
  <noscript>
    <input type="hidden" value="true" name="ssr" />
  </noscript>
</form>

<style>
  .qwant-search-form { display: flex; width: 100%; justify-content: center; font-family: var(--font-body); }
  .qwant-search-field { position: relative; display: grid; width: 100%; grid-template-columns: auto minmax(0, 1fr) auto; align-items: center; min-height: 58px; border: var(--rule) solid var(--color-rule); border-radius: var(--radius-pill); background: var(--color-paper-raised); box-shadow: var(--shadow-card); transition: border-color var(--dur-fast) var(--ease-out), box-shadow var(--dur-fast) var(--ease-out); }
  .qwant-search-field:focus-within { border-color: var(--color-accent); box-shadow: 0 0 0 4px var(--color-accent-soft), var(--shadow-card); }
  .qwant-search-field.has-suggestions { border-bottom-left-radius: var(--radius-lg); border-bottom-right-radius: var(--radius-lg); }
  .search-glyph { width: 20px; margin-left: var(--space-5); color: var(--color-muted); } .search-input { min-width: 0; border: 0; background: transparent; box-shadow: none !important; color: var(--color-ink); font: 500 16px var(--font-body); outline: 0; padding: var(--space-4); } .search-input:focus { box-shadow: none !important; } .search-input::placeholder { color: var(--color-muted); opacity: 1; }
  .search-submit { display: grid; place-items: center; width: 42px; height: 42px; margin-right: var(--space-2); border: 0; border-radius: 50%; background: var(--color-accent); color: var(--color-accent-ink); cursor: pointer; transition: background var(--dur-fast) var(--ease-out), transform var(--dur-fast) var(--ease-out); } .search-submit :global(svg) { width: 18px; } .search-submit:hover { background: var(--color-accent-strong); } .search-submit:active { transform: scale(.94); } .search-submit:focus-visible { outline: 3px solid var(--color-focus); outline-offset: 3px; }
  .suggestion-divider { position: absolute; inset-inline: var(--space-5); bottom: -1px; height: var(--rule); background: var(--color-rule); } .suggestion-divider.is-hidden { display: none; }
  .suggestions-panel { position: absolute; inset-inline: -1px; top: 100%; z-index: 20; overflow: clip; border: var(--rule) solid var(--color-rule); border-top: 0; border-radius: 0 0 var(--radius-lg) var(--radius-lg); background: var(--color-paper-raised); box-shadow: var(--shadow-float); } .suggestions-panel.is-hidden { display: none; }
  .suggestion-row { display: flex; width: 100%; align-items: center; gap: var(--space-3); border: 0; background: transparent; color: var(--color-ink); cursor: pointer; font: 500 14px var(--font-body); padding: var(--space-3) var(--space-5); text-align: left; } .suggestion-row:hover, .suggestion-row.is-selected { background: var(--color-paper-soft); } .suggestion-icon { width: 16px; color: var(--color-muted); }
  @media (max-width: 640px) { .qwant-search-field { min-height: 52px; } .search-glyph { margin-left: var(--space-4); } .search-input { font-size: 15px; padding-inline: var(--space-3); } .search-submit { width: 38px; height: 38px; } }
  @media (prefers-reduced-motion: reduce) { .qwant-search-field, .search-submit { transition-duration: var(--dur-fast); } }
</style>
