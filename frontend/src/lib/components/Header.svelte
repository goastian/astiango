<script lang="ts">
  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import LanguageSelect from './LanguageSelect.svelte';
  import { localizedHref } from '../../../../packages/i18n';
  import Bars2 from '~icons/heroicons/bars-2-20-solid';
  import { page } from '$app/stores';
  let open = false;
</script>

<nav class="qwant-nav" aria-label={$t("Primary navigation")}>
  <a class="skip" href="#main">{$t("Skip to content")}</a>
  <a href={localizedHref('/', $locale)} class="brand" aria-label={$t("AstianGO home")}><span>Astian</span><b>GO</b></a>
  <div class="desktop-links">
    <a class:active={$page.url.pathname === '/search'} href={localizedHref('/search', $locale)}>{$t("Search")}</a>
    <a class:active={$page.url.pathname === '/explore'} href={localizedHref('/explore', $locale)}>{$t("Explore")}</a>
    <a class:active={$page.url.pathname.startsWith('/settings')} href={localizedHref('/settings', $locale)}>{$t("Settings")}</a>
  </div>
  <LanguageSelect />
  <button class="menu" type="button" aria-label={$t("Show menu")} aria-expanded={open} on:click={() => (open = !open)}><Bars2 /></button>
  {#if open}
    <div class="mobile-menu"><a href={localizedHref('/search', $locale)}>{$t("Search")}</a><a href={localizedHref('/explore', $locale)}>{$t("Explore")}</a><a href={localizedHref('/settings', $locale)}>{$t("Settings")}</a><a href={localizedHref('/about', $locale)}>{$t("About")}</a></div>
  {/if}
</nav>

<style>
  .qwant-nav { position: relative; z-index: 10; display: flex; width: min(calc(100% - var(--space-6)), 1240px); align-items: center; justify-content: space-between; min-height: 56px; margin: var(--space-3) auto 0; padding: 0 var(--space-5); background: color-mix(in oklch, var(--color-paper) 94%, transparent); border: var(--rule) solid var(--color-rule); border-radius: var(--radius-pill); box-shadow: var(--shadow-card); backdrop-filter: blur(12px); }
  .skip { position: absolute; left: var(--space-4); top: calc(var(--space-12) * -2); padding: var(--space-2) var(--space-3); background: var(--color-ink); color: var(--color-paper); border-radius: var(--radius-sm); font: 700 13px var(--font-body); text-decoration: none; } .skip:focus { top: var(--space-2); }
  .brand { color: var(--color-accent); font: 700 25px/1 var(--font-display); letter-spacing: -.08em; text-decoration: none; } .brand b { color: var(--color-coral); } .desktop-links { display: flex; align-items: center; gap: var(--space-5); } .desktop-links a, .mobile-menu a { color: var(--color-ink-soft); font: 600 13px var(--font-body); text-decoration: none; white-space: nowrap; } .desktop-links a:hover, .desktop-links a.active { color: var(--color-accent-strong); }
  .menu { display: none; place-items: center; width: 38px; height: 38px; border: var(--rule) solid var(--color-rule); border-radius: 50%; background: var(--color-paper-raised); color: var(--color-ink); cursor: pointer; } .menu:hover { background: var(--color-paper-soft); } .menu:active { transform: scale(.94); } .menu:focus-visible, a:focus-visible { outline: 3px solid var(--color-focus); outline-offset: 3px; }
  .mobile-menu { position: absolute; right: 0; top: calc(100% + var(--space-2)); display: grid; gap: var(--space-3); min-width: 150px; padding: var(--space-4); border: var(--rule) solid var(--color-rule); border-radius: var(--radius-md); background: var(--color-paper-raised); box-shadow: var(--shadow-float); }
  @media (max-width: 640px) { .qwant-nav :global(.language-select > span) { display: none; } .desktop-links { display: none; } .menu { display: grid; } }
</style>
