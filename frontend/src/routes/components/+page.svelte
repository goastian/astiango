<script lang="ts">
  import { getI18n } from "$lib/i18n";
  const { t, locale } = getI18n();

  import IconGitHub from '~icons/simple-icons/github';
  import IconDiscord from '~icons/simple-icons/discord';
  import HandThumbUp from '~icons/heroicons/hand-thumb-up-20-solid';
  import Button from '$lib/components/Button.svelte';
  import Callout from '$lib/components/Callout.svelte';
  import Link from '$lib/components/Link.svelte';
  import { THEMES } from '$lib/themes';
  import { themeStore } from '$lib/stores';
</script>

<main class="relative mx-auto flex w-full max-w-2xl flex-col space-y-10 py-10">
  <div
    class="fixed translate-x-[calc(-100%-5rem)] space-y-2 rounded border bg-base-200 px-2 py-2 shadow"
  >
    <h1 class="text-2xl font-semibold">{$t("Themes")}</h1>
    <div class="flex flex-col space-y-1">
      {#each THEMES as theme}
        <button
          class="flex space-x-2 p-1 {theme.class} items-center rounded border bg-base-100 text-base-content"
          on:mouseenter={() => ($themeStore = theme.class)}
        >
          <span class="inline-flex flex-1 justify-center px-2 text-center font-bold">
            <span>
              {$t(theme.name)}
            </span>
          </span>
          <div class="flex overflow-hidden rounded border [&>*]:aspect-square [&>*]:w-10">
            <div class="bg-primary"></div>
            <div class="bg-secondary"></div>
            <div class="bg-accent"></div>
          </div>
        </button>
      {/each}
    </div>
  </div>

  <section class="flex flex-col space-y-2">
    <h1 class="text-3xl font-semibold">{$t("Buttons")}</h1>
    <div class="flex flex-wrap items-center gap-x-2 rounded-xl border p-4 shadow-xl">
      <Button>{$t("click me")}</Button>
      <Button padding={false}><HandThumbUp /></Button>
      <Button kind="seconday">{$t("click me")}</Button>
      <Button kind="seconday" padding={false}><HandThumbUp /></Button>
      <Button kind="accent">{$t("click me")}</Button>
      <Button kind="accent" padding={false}><HandThumbUp /></Button>
      <Button kind="neutral">{$t("click me")}</Button>
      <Button kind="neutral" padding={false}><HandThumbUp /></Button>
      <hr class="my-2 w-full" />
      <Button pale>{$t("click me")}</Button>
      <Button pale padding={false}><HandThumbUp /></Button>
      <Button pale kind="seconday">{$t("click me")}</Button>
      <Button pale kind="seconday" padding={false}><HandThumbUp /></Button>
      <Button pale kind="accent">{$t("click me")}</Button>
      <Button pale kind="accent" padding={false}><HandThumbUp /></Button>
      <Button pale kind="neutral">{$t("click me")}</Button>
      <Button pale kind="neutral" padding={false}><HandThumbUp /></Button>
      <hr class="my-2 w-full" />
      <Button kind="info">{$t("click me")}</Button>
      <Button kind="info" padding={false}><HandThumbUp /></Button>
      <Button kind="success">{$t("click me")}</Button>
      <Button kind="success" padding={false}><HandThumbUp /></Button>
      <Button kind="warning">{$t("click me")}</Button>
      <Button kind="warning" padding={false}><HandThumbUp /></Button>
      <Button kind="error">{$t("click me")}</Button>
      <Button kind="error" padding={false}><HandThumbUp /></Button>
      <hr class="my-2 w-full" />
      <Button pale kind="info">{$t("click me")}</Button>
      <Button pale kind="info" padding={false}><HandThumbUp /></Button>
      <Button pale kind="success">{$t("click me")}</Button>
      <Button pale kind="success" padding={false}><HandThumbUp /></Button>
      <Button pale kind="warning">{$t("click me")}</Button>
      <Button pale kind="warning" padding={false}><HandThumbUp /></Button>
      <Button pale kind="error">{$t("click me")}</Button>
      <Button pale kind="error" padding={false}><HandThumbUp /></Button>
    </div>
  </section>
  <section class="flex flex-col space-y-2">
    <h1 class="text-3xl font-semibold">{$t("Links")}</h1>
    <!-- svelte-ignore a11y-invalid-attribute -->
    <div class="rounded-xl border p-4 shadow-xl">
      <div class="flex items-center space-x-2">
        <Link href="#">{$t("This is a")}</Link>
        <Link href="#">{$t("Link to somewhere")}</Link>
        <Link href="#" round><IconDiscord /></Link>
        <Link href="#" round><IconGitHub /></Link>
        <a href="#" class="inline-flex pl-4 text-link">{$t("I haven't been here")}</a>
      </div>
      <hr class="my-2" />
      <div class="flex items-center space-x-2">
        <Link href="#" active>{$t("This is a")}</Link>
        <Link href="#" active>{$t("Link to somewhere")}</Link>
        <Link href="#" active round><IconDiscord /></Link>
        <Link href="#" active round><IconGitHub /></Link>
        <a href="#" class="inline-flex pl-4 text-link-visited">{$t("I have been here")}</a>
      </div>
    </div>
  </section>
  <section class="flex flex-col space-y-2">
    <h1 class="text-3xl font-semibold">{$t("Callouts")}</h1>
    <div class="space-y-4 rounded-xl border p-4 shadow-xl">
      <Callout kind="info">{$t("This is a info callout")}</Callout>
      <Callout kind="success">{$t("This is a success callout")}</Callout>
      <Callout kind="warning">{$t("This is a warning callout")}</Callout>
      <Callout kind="error">{$t("This is a error callout")}</Callout>
      <Callout kind="neutral">{$t("This is a neutral callout")}</Callout>
    </div>
  </section>
  <section class="flex flex-col space-y-2">
    <h1 class="text-3xl font-semibold">{$t("Colors")}</h1>
    <div class="flex flex-wrap gap-4 rounded-xl border p-4 shadow-xl">
      <div class="color-sample">
        <h2>{$t("primary")}</h2>
        <div>
          <div class="bg-primary"></div>
          <div class="bg-primary-focus"></div>
          <div class="bg-primary-content"></div>
        </div>
      </div>
      <div class="color-sample">
        <h2>{$t("secondary")}</h2>
        <div>
          <div class="bg-secondary"></div>
          <div class="bg-secondary-focus"></div>
          <div class="bg-secondary-content"></div>
        </div>
      </div>
      <div class="color-sample">
        <h2>{$t("accent")}</h2>
        <div>
          <div class="bg-accent"></div>
          <div class="bg-accent-focus"></div>
          <div class="bg-accent-content"></div>
        </div>
      </div>
      <div class="color-sample">
        <h2>{$t("neutral")}</h2>
        <div>
          <div class="bg-neutral"></div>
          <div class="bg-neutral-focus"></div>
          <div class="bg-neutral-content"></div>
        </div>
      </div>
      <div class="color-sample">
        <h2>{$t("base")}</h2>
        <div>
          <div class="bg-base-100"></div>
          <div class="bg-base-200"></div>
          <div class="bg-base-300"></div>
          <div class="bg-base-content"></div>
        </div>
      </div>
      <div class="color-sample">
        <h2>{$t("info")}</h2>
        <div>
          <div class="bg-info"></div>
          <div class="bg-info-focus"></div>
          <div class="bg-info-content"></div>
        </div>
      </div>
      <div class="color-sample">
        <h2>{$t("success")}</h2>
        <div>
          <div class="bg-success"></div>
          <div class="bg-success-focus"></div>
          <div class="bg-success-content"></div>
        </div>
      </div>
      <div class="color-sample">
        <h2>{$t("warning")}</h2>
        <div>
          <div class="bg-warning"></div>
          <div class="bg-warning-focus"></div>
          <div class="bg-warning-content"></div>
        </div>
      </div>
      <div class="color-sample">
        <h2>{$t("error")}</h2>
        <div>
          <div class="bg-error"></div>
          <div class="bg-error-focus"></div>
          <div class="bg-error-content"></div>
        </div>
      </div>
      <div class="color-sample">
        <h2>{$t("link")}</h2>
        <div>
          <div class="bg-link"></div>
          <div class="bg-link-visited"></div>
        </div>
      </div>
    </div>
  </section>
</main>

<style lang="postcss">
  .color-sample h2 {
    @apply mb-1 text-lg font-semibold capitalize;
  }
  .color-sample > div {
    @apply inline-flex overflow-hidden rounded border border-base-300 [&>*]:aspect-square [&>*]:w-8;
  }
</style>
