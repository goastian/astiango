<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { ExtractionStats } from "./types";
  let { stats, readError, stale }: { stats?: ExtractionStats; readError: boolean; stale: boolean } = $props();
  const number = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 2 });
  const display = (value: number | undefined) => value == null ? "—" : number.format(value);
  const groups = $derived([
    { title: "Extracción compartida", metrics: [
      { label: "Documentos procesados", value: stats?.documents },
      { label: "Parseos DOM", value: stats?.dom_parses },
      { label: "DOM por documento", value: stats?.documents ? stats.dom_parses / stats.documents : undefined },
      { label: "Fallos de extracción", value: stats?.failures },
      { label: "CPU por documento", value: stats?.cpu_samples ? stats.cpu_us / stats.cpu_samples / 1000 : undefined, unit: "ms" },
      { label: "Tiempo por documento", value: stats?.documents ? stats.wall_us / stats.documents / 1000 : undefined, unit: "ms" },
    ] },
    { title: "Enlaces y límites", metrics: [
      { label: "Anchors recorridos", value: stats?.anchors_visited },
      { label: "Enlaces para descubrimiento", value: stats?.discovery_links },
      { label: "Enlaces internos admitidos", value: stats?.internal_links },
      { label: "Límite por proyección", value: stats?.max_links_per_projection },
      { label: "Páginas en el límite de descubrimiento", value: stats?.pages_at_discovery_limit },
      { label: "Páginas en el límite interno", value: stats?.pages_at_internal_limit },
      { label: "Recorridos cortados al completar cuotas", value: stats?.traversals_stopped },
    ] },
  ]);
</script>

{#if stats}
  <details class="panel extraction-panel">
    <summary><strong>Enlaces y metadatos</strong>
      <span class="extraction-summary">{display(stats.documents)} documentos · {display(stats.dom_parses)} parseos DOM</span>
      <span class="extraction-toggle">Detalles <Icon name="chevron" size={16} /></span>
    </summary>
    <div class="extraction-details">
      <p>Una extracción comparte los enlaces de exploración y descubrimiento. Respeta base URL y robots meta.
        Los límites se aplican durante la extracción; la CPU corresponde al análisis de HTML y enlaces de esta ejecución.
        Las asignaciones por página y la comparación anterior se publican en el informe local de extracción.</p>
      {#if readError || stale}<p class="extraction-notice">Se conserva la última lectura disponible de enlaces y metadatos.</p>{/if}
      <div class="extraction-groups">{#each groups as group}<section aria-label={group.title}>
        <h3>{group.title}</h3><dl>{#each group.metrics as metric}<div>
          <dt>{metric.label}</dt><dd>{display(metric.value)}{metric.value != null && metric.unit ? ` ${metric.unit}` : ""}</dd>
        </div>{/each}</dl>
      </section>{/each}</div>
    </div>
  </details>
{/if}

<style>
  .extraction-panel { margin-top: 24px; }
  summary { display: flex; align-items: center; flex-wrap: wrap; gap: 8px 16px; padding: 16px 24px; cursor: pointer; font-size: 12px; }
  .extraction-summary, dt { color: #687381; }
  .extraction-toggle { display: inline-flex; align-items: center; gap: 3px; margin-left: auto; color: #687381; }
  .extraction-panel[open] .extraction-toggle :global(svg) { transform: rotate(180deg); }
  .extraction-details { padding: 0 24px 22px; font-size: 12px; line-height: 1.6; }
  .extraction-details > p { max-width: 980px; }
  .extraction-notice { color: #916215; }
  .extraction-groups { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 28px; margin-top: 22px; }
  h3 { font-size: 13px; }
  dl { display: grid; gap: 12px; margin: 14px 0 0; }
  dl > div { display: flex; justify-content: space-between; gap: 16px; border-bottom: 1px solid var(--line); padding-bottom: 8px; }
  dd { margin: 0; font-weight: 600; }
  @media (max-width: 960px) { .extraction-groups { grid-template-columns: 1fr; gap: 20px; } }
  @media (max-width: 620px) { summary { padding: 16px 20px; } .extraction-details { padding: 0 20px 20px; } .extraction-summary { order: 1; width: 100%; } }
</style>
