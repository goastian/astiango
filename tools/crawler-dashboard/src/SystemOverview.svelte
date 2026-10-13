<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import type { Snapshot } from "./types";
  import Icon from "./Icon.svelte";
  import { componentCatalog } from "./navigation";
  let { snapshot, connected, status, inventory = false }: {
    snapshot?: Snapshot; connected: boolean; status: string; inventory?: boolean;
  } = $props();
  const format = $derived(new Intl.NumberFormat($formatLocale));
  const number = (value?: number) => value === undefined ? "—" : format.format(value);
</script>

{#if !inventory}
  <section class="system-spotlight panel" aria-label={$t("Componente conectado")}>
    <div class="spotlight-icon"><Icon name="globe" size={28} /></div>
    <div><span class="eyebrow">{$t("Adquisición web")}</span><h2>{$t("Crawler")}</h2><p>{$t("El rastreo publica su estado en esta instancia. Consulta producción, sitios y recursos desde sus vistas de trabajo.")}</p></div>
    <a class="button primary" href="#/crawler/overview">{$t("Abrir crawler")} <Icon name="arrow" size={16} /></a>
  </section>
  <section class="system-metrics" aria-label={$t("Totales del crawler conectado")}>
    <article class="panel"><span>{$t("Páginas guardadas")}</span><strong>{number(snapshot?.counters.downloaded)}</strong><small>{$t("En esta ejecución")}</small></article>
    <article class="panel"><span>{$t("URLs pendientes")}</span><strong>{number(snapshot?.frontier_db?.pending_urls)}</strong><small>{$t("Cola durable")}</small></article>
    <article class="panel"><span>{$t("Capturas durables")}</span><strong>{number(snapshot?.spool?.records_durable)}</strong><small>{$t("Incluye ejecuciones anteriores")}</small></article>
  </section>
{/if}

<section class="panel component-directory" aria-label={$t("Integración de componentes")}>
  <div class="section-heading"><h2>{inventory ? $t("Fuentes de estado") : $t("Componentes del buscador")}</h2>{#if !inventory}<a href="#/components">{$t("Ver integraciones")} <Icon name="arrow" size={15} /></a>{/if}</div>
  <p class="directory-note">{$t("Solo el crawler tiene una fuente de estado conectada. La ausencia de integración no indica que un servicio esté detenido.")}</p>
  <div class="component-rows">
    {#each componentCatalog as component}
      <article class="component-row">
        <span class="component-symbol"><Icon name={component.source ? "globe" : "components"} size={19} /></span>
        <div><h3>{$t(component.name)}</h3><p>{$t(component.purpose)}</p></div>
        <span class="integration-state" class:source-connected={component.source && connected}>{component.source ? $t(status) : $t("Sin integración con el panel")}</span>
        {#if component.source}<a href="#/crawler/overview" aria-label={$t("Consultar crawler")}><Icon name="arrow" size={18} /></a>{/if}
      </article>
    {/each}
  </div>
</section>
