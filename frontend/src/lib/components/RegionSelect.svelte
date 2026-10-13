<script lang="ts">
  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import { type Region, REGIONS } from '$lib/api';
  import Select from './Select.svelte';

  export let searchOnChange: boolean;
  export let selected: Region = REGIONS[0];

  const codes: Record<string, string> = { Denmark: 'DK', France: 'FR', Germany: 'DE', Spain: 'ES', US: 'US' };
  $: options = REGIONS.map((region) => ({
    value: region,
    label: region == 'All' ? 'All Languages' : new Intl.DisplayNames([$locale], { type: 'region' }).of(codes[region]) ?? region,
  }));
</script>

<div class="m-0 flex h-full flex-col justify-center p-0">
  <Select
    form="searchbar-form"
    id="region-selector"
    name="gl"
    class="m-0 cursor-pointer text-xs text-neutral-focus"
    submitOnChange={searchOnChange}
    bind:value={selected}
    {options}
    ariaLabel={$t("Region selector")}
  />
</div>
