<script lang="ts">
  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import { DEFAULT_OPTICS, opticKey } from '$lib/optics';
  import { opticsStore, opticsShowStore } from '$lib/stores';
  import Select from './Select.svelte';
  import { derived } from 'svelte/store';

  export let searchOnChange: boolean;
  export let selected = '';

  for (const optic of DEFAULT_OPTICS) {
    if ($opticsShowStore[opticKey(optic)] !== undefined) continue;

    opticsShowStore.update(($opticsShow) => ({
      ...$opticsShow,
      [opticKey(optic)]: optic.shown,
    }));
  }

  const optics = derived(opticsStore, ($optics) => [...$optics, ...DEFAULT_OPTICS]);

  $: options = [
    { value: '', label: $t('No Optic') },
    ...$optics
      .filter((optic) => $opticsShowStore[opticKey(optic)])
      .map((optic) => ({ value: optic.url, label: DEFAULT_OPTICS.some(item => item.url === optic.url) ? $t(optic.name) : optic.name })),
  ];
</script>

<div class="noscript:hidden m-0 flex h-full flex-col justify-center p-0">
  <Select
    translateLabels={false}
    form="searchbar-form"
    id="optics-selector"
    name="optic"
    class="m-0 cursor-pointer text-xs text-neutral-focus"
    submitOnChange={searchOnChange}
    bind:value={selected}
    {options}
    ariaLabel={$t("Optic selector")}
  />
</div>
