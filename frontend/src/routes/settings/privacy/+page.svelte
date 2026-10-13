<script lang="ts">
  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import { allowStatsStore } from '$lib/stores';
</script>

<div class="flex flex-col space-y-3 [&_a]:font-medium [&_a]:underline">
  <h1 class="text-2xl font-medium">{$t("Privacy")}</h1>
  <div class="text-sm">
    {$t("By default, some data about your searches are stored on our servers. We primarily store the text you used for the search, and which results (if any) you clicked on. This data immensely helps us improve everyones search experience by training our ranking models.")}
  </div>

  <div class="text-sm">
    {$t("It is important to note, that")} <b class="font-extrabold">{$t("no")}</b>{$t("{p0} personal information is stored about your search. We don't store your IP, browser fingerprint or even a precise timestamp. In fact, you can see exactly what gets stored{p1}", { p0: ' ', p1: ' ' })}
    <a href="https://github.com/goastian/astiango/blob/main/crates/core/src/improvement.rs#L37-L51"
      >{$t("here")}</a
    >.
  </div>

  <div class="text-sm">
    {$t("We hope you feel comfortable sharing this data with us as this makes the results much better for everyone, but we also respect if you don't. You can opt-out at any point by toggling the checkbox below and we wont store anything at all about your searches.")}
  </div>

  <div class="noscript:hidden flex space-x-1 pl-1 text-sm">
    <input type="checkbox" id="allow-stats" bind:checked={$allowStatsStore} />
    <label for="allow-stats">{$t("Allow usage statistics")}</label>
  </div>
</div>
