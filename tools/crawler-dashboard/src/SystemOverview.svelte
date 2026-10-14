<script lang="ts">
  import { getI18n } from "./i18n";
  import Icon from "./Icon.svelte";
  import TimeChart from "./TimeChart.svelte";
  import type { Snapshot, IndexingResponse, WebgraphResponse } from "./types";
  import { componentCatalog } from "./navigation";
  import { consumerActivity } from "./work-status";
  const { t, formatLocale } = getI18n();
  let { snapshot, connected, status, indexing = null, indexingLive = false, webgraph = null, webgraphLive = false, readingPaused = false, inventory = false }: {
    snapshot?: Snapshot; connected: boolean; status: string; indexing?: IndexingResponse | null; indexingLive?: boolean; webgraph?: WebgraphResponse | null; webgraphLive?: boolean; readingPaused?: boolean; inventory?: boolean;
  } = $props();
  const format = $derived(new Intl.NumberFormat($formatLocale));
  const number = (v?: number) => v === undefined ? "—" : format.format(v);
  const components = $derived([
    {name:"Crawler",href:"#/crawler/overview",icon:"globe",live:connected,label:"Capturas durables",value:snapshot?.spool?.records_durable,description:"Incluye ejecuciones anteriores",phase:status},
    {name:"Indexing",href:"#/indexing/overview",icon:"index",live:indexingLive,label:"Active documents",value:indexing?.snapshot.totals.indexed_documents,description:"Current URL versions",phase:consumerActivity(indexing?.snapshot,snapshot).label},
    {name:"Webgraph",href:"#/webgraph/overview",icon:"graph",live:webgraphLive,label:"Stored links",value:webgraph?.snapshot.totals.stored_edges,description:webgraph?.snapshot.scope === "current_version_graph" ? "Current graph versions" : "Per-input graphs; not global unique links",phase:consumerActivity(webgraph?.snapshot,snapshot).label},
  ]);
</script>

{#if !inventory}
  <section class="component-flow" aria-label={$t("Integración de componentes")}>
    {#each components as component}
      <a href={component.href} class="component-step"><header><Icon name={component.icon} size={20}/><h2>{$t(component.name)}</h2><span class:online={component.live}>{$t(readingPaused ? "Actualización pausada." : component.live ? component.phase : "State unavailable or stale")}</span></header><span class="metric-label">{$t(component.label)}</span><strong>{number(component.value)}</strong><small>{$t(component.description)}</small></a>
    {/each}
  </section>
  {#if connected && snapshot?.resources?.reason === "spool_full"}
    <aside class="pipeline-block" role="status"><Icon name="storage" size={18}/><div><strong>{$t("Crawler segment quota reached")}</strong><span>{$t("Retained segments")}: {number(snapshot.spool?.segments_total)} / {number(snapshot.spool?.max_segments)} · {$t("Spool usage")}: {number(Math.round((snapshot.spool?.spool_bytes ?? 0)/1048576))} MiB</span></div><a href="#/crawler/storage">{$t("Inspect storage")} →</a></aside>
  {/if}
  <section class="system-charts"><article class="panel"><header><h2>{$t("Active documents")}</h2><a href="#/indexing/overview">{$t("Indexing")} →</a></header><TimeChart points={(indexing?.snapshot.history ?? []).map(v => ({at:v.at,value:v.documents}))} label="Active documents"/></article><article class="panel"><header><h2>{$t("Stored links")}</h2><a href="#/webgraph/overview">{$t("Webgraph")} →</a></header><TimeChart points={(webgraph?.snapshot.history ?? []).map(v => ({at:v.at,value:v.edges}))} label="Stored links" color="#596f87"/></article></section>
{/if}
<section class="panel component-directory" aria-label={$t("Integración de componentes")}>
  <div class="section-heading"><h2>{inventory ? $t("Fuentes de estado") : $t("Componentes del buscador")}</h2>{#if !inventory}<a href="#/components">{$t("Ver integraciones")} <Icon name="arrow" size={15}/></a>{/if}</div>
  <div class="component-rows">
    {#each componentCatalog as component}
      {@const link = components.find(v => v.name === (component.source === "indexer" ? "Indexing" : component.source === "webgraph" ? "Webgraph" : "Crawler"))}
      <article class="component-row"><span class="component-symbol"><Icon name={component.source === "webgraph" ? "graph" : component.source === "indexer" ? "index" : "components"} size={19}/></span><div><h3>{$t(component.name)}</h3>{#if inventory}<p>{$t(component.purpose)}</p>{/if}</div><span class="integration-state" class:source-connected={!!component.source && !!link?.live}>{$t(component.source ? readingPaused ? "Actualización pausada." : link?.live ? "Connected" : "State unavailable or stale" : "Sin integración con el panel")}</span>{#if component.source && link}<a href={link.href} aria-label={$t(component.name)}><Icon name="arrow" size={18}/></a>{/if}</article>
    {/each}
  </div>
</section>
<style>
  .pipeline-block {display:flex;align-items:center;gap:14px;padding:16px 20px;border:1px solid #e8d5ac;background:#fffbf2;color:#875e21;}
  .pipeline-block div {flex:1;display:grid;gap:5px;font-size:12px;}.pipeline-block strong {font-size:13px;}.pipeline-block a {color:inherit;font-size:12px;}
  .component-flow{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:1px;background:#dce4de;border:1px solid #dce4de;margin-bottom:22px}.component-step{padding:24px;background:#fafcf9;color:inherit;text-decoration:none;min-width:0}.component-step:hover{background:#eef5ee}header{display:flex;align-items:center;gap:10px;flex-wrap:wrap}h2{font-size:15px;margin:0}header>span{margin-left:auto;font-size:11px;color:#956a20}header>span.online{color:#326d50}.metric-label{display:block;margin-top:28px;font-size:12px;color:#66776b}.component-step>strong{display:block;font-size:42px;letter-spacing:-1.5px;margin:10px 0;font-variant-numeric:tabular-nums}.component-step small{font-size:11px;color:#78857b;display:block;max-width:38ch}.system-charts{display:grid;grid-template-columns:1fr 1fr;gap:18px;margin-bottom:22px}.system-charts .panel{padding:24px;min-width:0}.system-charts header{justify-content:space-between;margin-bottom:14px}.system-charts a{font-size:12px;color:#326d50}.component-step:focus-visible{outline:2px solid #326d50;outline-offset:3px}@media(max-width:900px){.component-flow{grid-template-columns:1fr}.component-step{padding:20px}.metric-label{margin-top:16px}.component-step>strong{font-size:34px}.system-charts{grid-template-columns:1fr}}
</style>
