<svelte:options accessors />
<script lang="ts">
  import { languageParameter } from '../../../../packages/i18n';
  import { getI18n } from '$lib/i18n';
  import { safeSearchStore } from '$lib/stores';
  import { page } from '$app/stores';
  const { t, locale } = getI18n();
  export let autofocus = false;
  export let query = '';
  let formElem: HTMLFormElement;
  let inputElem: HTMLInputElement;
  let composing = false;
  export const getInputElem = () => inputElem;
  export const getForm = () => formElem;
  export const select = () => inputElem?.select();
  export const userQuery = () => query;
  export const search = (q: string) => { query=q; if(inputElem)inputElem.value=q;formElem?.requestSubmit(); };
  $: safe = $page.url.searchParams.has('ss') ? $page.url.searchParams.get('ss') === 'true' : $safeSearchStore;
</script>
<form action="/search" method="GET" class="search-form" role="search" bind:this={formElem} on:submit={(event)=>{if(composing || !query.trim())event.preventDefault();}}>
  <input type="hidden" name={languageParameter} value={$locale} />
  <input type="hidden" name="ss" value={safe ? 'true' : 'false'} />
  <div class="search-field">
    <label for="searchbar" class="sr-only">{$t('Search')}</label>
    <!-- svelte-ignore a11y_autofocus -->
    <input id="searchbar" name="q" type="search" {autofocus} required maxlength="4096" placeholder={$t('Search the web privately')} autocomplete="off" class="search-input" bind:value={query} bind:this={inputElem} on:compositionstart={()=>composing=true} on:compositionend={()=>composing=false} />
    <button class="search-submit" type="submit" aria-label={$t('Search')}><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" aria-hidden="true"><circle cx="10.5" cy="10.5" r="6.5"/><path d="m16 16 5 5"/></svg></button>
  </div>
</form>
<style>
  .search-form{width:100%;font-family:var(--font-body)}.search-field{display:flex;align-items:center;min-height:56px;border:1px solid var(--color-rule);border-radius:var(--radius-pill);background:var(--color-paper-raised);box-shadow:var(--shadow-card);padding:4px 8px 4px 24px}.search-field:focus-within{border-color:var(--color-accent);box-shadow:0 0 0 3px var(--color-accent-soft)}.search-input{width:100%;min-width:0;background:transparent;border:0;color:var(--color-ink);font:400 16px/1.5 var(--font-body);padding:8px 0;box-shadow:none!important;outline:none}.search-input::placeholder{color:var(--color-muted)}.search-submit{display:grid;place-items:center;flex-shrink:0;width:44px;height:44px;border:0;border-radius:50%;background:var(--color-accent);color:var(--color-accent-ink);cursor:pointer;margin-left:8px}.search-submit:hover{background:var(--color-accent-strong)}.search-submit svg{width:22px;height:22px}.search-submit:focus-visible{outline:3px solid var(--color-focus);outline-offset:3px}@media(max-width:640px){.search-field{padding-left:16px}}
</style>
