<script lang="ts">
  import '../app.css';
  import '../../../packages/i18n/fonts.css';
  import { initI18n } from '$lib/i18n';
  import type { LayoutData } from './$types';
  export let data: LayoutData;
  const { t, locale } = initI18n(data.uiLocale);
  import Header from '$lib/components/Header.svelte';
  import Footer from '$lib/components/Footer.svelte';
  import { page } from '$app/stores';
</script>

<svelte:head>
  <title>{$t($page.data.globals?.title || 'AstianGO')}</title>
  <meta name="description" content={$t("A private, open and transparent search engine with an independent crawler and index.")} />
  <meta property="og:locale" content={$locale.replace("-", "_")} />
  <meta property="og:title" content="AstianGO" />
  <meta property="og:image" content="https://astiango.com/images/biglogo-with-text.png" />
  <meta name="viewport" content="width=device-width" />
</svelte:head>

<div class="grid min-h-screen grid-rows-[auto_1fr_auto] font-light">
  <Header />

  <main id="main" tabindex="-1">
    <slot />
  </main>

  <Footer />
</div>

<noscript>
  <style lang="postcss">
    .noscript\:hidden {
      @apply hidden;
    }
  </style>
</noscript>
