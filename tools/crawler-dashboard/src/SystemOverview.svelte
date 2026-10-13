<script lang="ts">
  import type { Snapshot } from "./types";
  import Icon from "./Icon.svelte";
  import { componentCatalog } from "./navigation";
  let { snapshot, connected, status, inventory = false }: {
    snapshot?: Snapshot; connected: boolean; status: string; inventory?: boolean;
  } = $props();
  const format = new Intl.NumberFormat("es-CO");
  const number = (value?: number) => value === undefined ? "—" : format.format(value);
</script>

{#if !inventory}
  <section class="system-spotlight panel" aria-label="Componente conectado">
    <div class="spotlight-icon"><Icon name="globe" size={28} /></div>
    <div><span class="eyebrow">Adquisición web</span><h2>Crawler</h2><p>El rastreo publica su estado en esta instancia. Consulta producción, sitios y recursos desde sus vistas de trabajo.</p></div>
    <a class="button primary" href="#/crawler/overview">Abrir crawler <Icon name="arrow" size={16} /></a>
  </section>
  <section class="system-metrics" aria-label="Totales del crawler conectado">
    <article class="panel"><span>Páginas guardadas</span><strong>{number(snapshot?.counters.downloaded)}</strong><small>En esta ejecución</small></article>
    <article class="panel"><span>URLs pendientes</span><strong>{number(snapshot?.frontier_db?.pending_urls)}</strong><small>Cola durable</small></article>
    <article class="panel"><span>Capturas durables</span><strong>{number(snapshot?.spool?.records_durable)}</strong><small>Incluye ejecuciones anteriores</small></article>
  </section>
{/if}

<section class="panel component-directory" aria-label="Integración de componentes">
  <div class="section-heading"><h2>{inventory ? "Fuentes de estado" : "Componentes del buscador"}</h2>{#if !inventory}<a href="#/components">Ver integraciones <Icon name="arrow" size={15} /></a>{/if}</div>
  <p class="directory-note">Solo el crawler tiene una fuente de estado conectada. La ausencia de integración no indica que un servicio esté detenido.</p>
  <div class="component-rows">
    {#each componentCatalog as component}
      <article class="component-row">
        <span class="component-symbol"><Icon name={component.source ? "globe" : "components"} size={19} /></span>
        <div><h3>{component.name}</h3><p>{component.purpose}</p></div>
        <span class="integration-state" class:source-connected={component.source && connected}>{component.source ? status : "Sin integración con el panel"}</span>
        {#if component.source}<a href="#/crawler/overview" aria-label="Consultar crawler"><Icon name="arrow" size={18} /></a>{/if}
      </article>
    {/each}
  </div>
</section>
