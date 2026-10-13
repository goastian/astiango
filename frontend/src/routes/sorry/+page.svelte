<script lang="ts">
  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import Speaker from '~icons/heroicons/speaker-wave-20-solid';
  import Button from '$lib/components/Button.svelte';
  import type { PageData } from './$types';
  import Visual from './Visual.svelte';
  import Captcha from './Captcha.svelte';
  import { page } from '$app/stores';

  export let data: PageData;
</script>

<Captcha resultDigestBase64={data.resultDigestBase64}>
  <Visual imgBase64={data.imgBase64} animal={data.animal} count={data.count} />

  <div class="flex w-full justify-between">
    <a href={`sorry/audio${$page.url.search}`} title={$t("Get an audio challenge")}
      ><Speaker class="h-8 w-8" /></a
    >
    <Button title={$t("Verify challenge")} type="submit" kind="accent">{$t("VERIFY")}</Button>
  </div>
</Captcha>
