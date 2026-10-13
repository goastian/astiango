<script lang="ts">
  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import Select from '$lib/components/Select.svelte';
  import { postSearchStore } from '$lib/stores';

  let options = [
    { value: void 0, label: 'GET' },
    { value: true, label: 'POST' },
  ];
</script>

<div class="noscript:hidden">
  <Select
    bind:value={$postSearchStore}
    class="text-neutral-focus"
    {options}
    ariaLabel={$t("Select request type")}
  />
</div>
