<script lang="ts">
  import { getI18n } from '$lib/i18n';
  import { opticsStore } from '$lib/stores';
  const {t}=getI18n();
</script>
<h1 class="text-2xl font-medium">{$t('Optics')}</h1>
<p class="my-4" role="status">{$t('Unavailable with the current search engine.')}</p>
<ul>{#each $opticsStore as optic}<li class="my-2">{optic.name}</li>{/each}</ul>
