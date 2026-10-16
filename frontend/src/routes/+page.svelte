<script lang="ts">
  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import Searchbar from '$lib/components/Searchbar.svelte';
</script>

<section class="search-home" aria-labelledby="home-title">
  <div class="home-content">
    <h1 id="home-title" class="home-wordmark"><span>Astian</span><b>GO</b></h1>
    <p class="home-lede">{$t("Search the open web on your terms.")}</p>
    <div class="home-search">
    <Searchbar autofocus={true} />
    </div>
  </div>
</section>

<style>
  .search-home { display: flex; width: 100%; min-height: 0; align-items: flex-start; justify-content: center; padding: clamp(var(--space-16), 17vh, var(--space-40)) var(--space-5) var(--space-8); background: var(--color-paper); }
  .home-content { display: flex; width: min(100%, 720px); flex-direction: column; align-items: center; } .home-wordmark { margin: 0; color: var(--color-accent); font: 700 clamp(4rem, 11vw, 6.5rem)/.86 var(--font-display); letter-spacing: -.09em; overflow-wrap: anywhere; } .home-wordmark b { color: var(--color-coral); font: inherit; } .home-lede { margin: var(--space-4) 0 var(--space-8); color: var(--color-ink-soft); font: 500 clamp(16px, 2vw, 19px)/1.4 var(--font-body); text-align: center; }
  .home-search { width: min(100%, 680px); }
  @media (max-width: 640px) { .search-home { padding-top: clamp(var(--space-16), 18vh, var(--space-24)); } }
</style>
