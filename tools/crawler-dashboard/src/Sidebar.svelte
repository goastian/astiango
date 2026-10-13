<script lang="ts">
  import Icon from "./Icon.svelte";
  import { views } from "./navigation";
  let { current, status, connected }: { current: string; status: string; connected: boolean } = $props();
</script>

<div class="sidebar-content">
  <a class="control-brand" href="#/overview" aria-label="AstianGO · Vista general">
    <span class="brand-mark"><Icon name="components" size={22} /></span>
    <span><strong>AstianGO</strong><small>Centro de control</small></span>
  </a>
  <nav aria-label="Navegación principal">
    {#each ["Sistema", "Crawler", "Herramientas"] as group}
      <div class="nav-group">
        <span class="nav-group-label">{group}</span>
        {#each views.filter(view => view.group === group) as view}
          <a href={`#${view.path}`} class:nav-current={current === view.id} aria-current={current === view.id ? "page" : undefined}>
            <Icon name={view.icon} size={18} /><span>{view.label}</span>
          </a>
        {/each}
      </div>
    {/each}
  </nav>
  <div class="sidebar-status">
    <span class="sidebar-indicator" class:connected></span>
    <div><strong>Instancia local</strong><small>{status}</small></div>
  </div>
</div>
