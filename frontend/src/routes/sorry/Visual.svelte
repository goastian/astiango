<script lang="ts">
  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import Patch from './Patch.svelte';
  import type { Animal } from '$lib/captcha/lib';

  export let imgBase64: string;
  export let animal: Animal;
  export let count: number;
</script>

<div>
  <p class="text-xl">{$t("Select {p0} images containing {p1}.", { p0: count, p1: $t(animal) })}</p>
  <div class="relative w-fit">
    <img src={imgBase64} alt={$t("captcha of animals")} />
    <div class="absolute bottom-0 left-0 top-0 h-full w-full">
      {#key imgBase64}
        <Patch id={0} />
        <Patch id={1} />
        <Patch id={2} />
        <Patch id={3} />
        <Patch id={4} />
        <Patch id={5} />
        <Patch id={6} />
        <Patch id={7} />
        <Patch id={8} />
      {/key}
    </div>
  </div>
</div>
