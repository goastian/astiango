<script lang="ts">
  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import { page } from '$app/stores';
</script>

<main class="grid h-full place-items-center">
  <div class="flex flex-col items-center justify-center text-center">
    <h1 class="text-4xl">{$t($page.error?.message ?? "")}</h1>
    <p class="text-xl"><span class="select-none text-transparent">...</span>{$t("An error occured...")}</p>
    <br />
    <a
      class="rounded bg-base-200 px-2 py-1 text-neutral"
      href={$page.url.toString()}
      on:click|preventDefault={() => location.reload()}>{$t("Try again")}</a
    >
  </div>
</main>
