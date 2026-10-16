<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale } = getI18n();

  import LanguageSelect from "./LanguageSelect.svelte";
  import Icon from "./Icon.svelte";
  import { views } from "./navigation";
  let { current, status, connected }: { current: string; status: string; connected: boolean } = $props();
</script>

<div class="sidebar-content">
  <a class="control-brand" href="#/overview" aria-label={$t("AstianGO · Vista general")}>
    <span class="brand-mark"><Icon name="components" size={22} /></span>
    <span><strong>AstianGO</strong><small>{$t("Centro de control")}</small></span>
  </a>
  <nav aria-label={$t("Navegación principal")}>
    {#each ["Sistema", "Crawler", "Indexing", "Webgraph", "Search & API", "Web frontend", "Herramientas"] as group}
      <details class="nav-group" open={views.find(view => view.id === current)?.group === group}>
        <summary class="nav-group-label">{$t(group)}</summary>
        {#each views.filter(view => view.group === group) as view}
          <a href={`#${view.path}`} class:nav-current={current === view.id} aria-current={current === view.id ? "page" : undefined}>
            <Icon name={view.icon} size={18} /><span>{$t(view.label)}</span>
          </a>
        {/each}
      </details>
    {/each}
  </nav>
  <div class="sidebar-language"><LanguageSelect /></div>
  <div class="sidebar-status">
    <span class="sidebar-indicator" class:connected></span>
    <div><strong>{$t("Instancia local")}</strong><small>{status}</small></div>
  </div>
</div>

<style>
  summary { cursor: pointer; list-style: none; display: flex !important; align-items: center; justify-content: space-between; }
  summary::-webkit-details-marker { display: none; }
  summary::after { content: "+"; font-size: 16px; font-weight: 400; }
  details[open]>summary::after { content: "−"; }
  summary:hover { color: #326d50; }
  summary:focus-visible { outline: 2px solid #326d50; outline-offset: 4px; }
</style>
