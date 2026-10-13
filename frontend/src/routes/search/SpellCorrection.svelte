<script lang="ts">
  import { localizedHref, languageParameter } from '../../../../packages/i18n';

  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import type { HighlightedSpellCorrection } from '$lib/api';

  export let spellCorrection: HighlightedSpellCorrection;

  let link: HTMLAnchorElement;
  export const open = () => {
    link.click();
  };
  export const hasFocus = () => document.activeElement == link;
</script>

<div>
  {$t("Did you mean:{p0}", { p0: ' ' })}
  <a
    class="font-medium"
    href={localizedHref(`/search?q=${encodeURIComponent(spellCorrection.raw)}`, $locale)}
    bind:this={link}
  >
    {#each spellCorrection.highlighted as frag}
      {#if frag.kind == 'highlighted'}
        <b><i>{frag.text}</i></b>
      {:else}
        {frag.text}
      {/if}
    {/each}
  </a>
</div>
