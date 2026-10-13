<script lang="ts">
  import { getI18n } from "./i18n";
  import { languages } from "../../../packages/i18n";
  const { locale, t, setLocale } = getI18n();
</script>

<label class="language-select">
  <span>{$t("Interface language")}</span>
  <select aria-label={$t("Interface language")} value={$locale} on:change={(event) => setLocale(event.currentTarget.value)}>
    {#each languages as language}
      <option value={language.code} lang={language.code} selected={language.code === $locale}>{language.name}</option>
    {/each}
  </select>
</label>

<style>
  .language-select { display: flex; flex-wrap: wrap; align-items: center; gap: .5rem; font: 500 12px/1.4 system-ui, sans-serif; color: inherit; }
  select { max-width: 100%; padding: .45rem .65rem; border: 1px solid #ccd5e1; border-radius: 6px; color: #263449; background: white; font: inherit; cursor: pointer; }
  select:focus-visible { outline: 3px solid #73aff4; outline-offset: 2px; }
</style>
