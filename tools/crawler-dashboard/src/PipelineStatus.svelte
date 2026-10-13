<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { PipelineStats } from "./types";
  let { stats, readError, stale }: { stats?: PipelineStats; readError: boolean; stale: boolean } = $props();
  const number = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 2 });
  const display = (value: number | undefined) => value == null ? "—" : number.format(value);
  const stages = $derived(stats ? [
    { title: "Admisión de páginas", value: stats.admission },
    { title: "Preparación HTTP", value: stats.network_setup },
    { title: "Parseo", value: stats.parse },
    { title: "Compresión y almacenamiento", value: stats.storage },
    { title: "Frontera y control", value: stats.control },
    ...(stats.batch_queue.peak_items ? [{ title: "Cola WARC batch", value: stats.batch_queue }] : []),
  ] : []);
</script>
{#if stats}
  <details class="panel pipeline-panel">
    <summary><strong>Pipeline y contrapresión</strong>
      <span class="pipeline-summary">{stats.runtime_lag_samples ? `Lag p99 ${display(stats.runtime_lag_p99_us / 1000)} ms` : "Esperando muestras de lag"} · {stats.accepting ? "Admisión abierta" : "Drenando"}</span>
      <span class="pipeline-toggle">Detalles <Icon name="chevron" size={16} /></span>
    </summary>
    <div class="pipeline-details">
      <p>HTTP continúa en tareas async; el parseo y la compresión/escritura usan trabajadores limitados.
        Las cuotas incluyen trabajos encolados y en ejecución. Los bytes corresponden a entradas reservadas;
        el RSS también incluye DOM, buffers, conexiones y estado de la frontera. Un trabajo bloqueante iniciado
        conserva sus recursos hasta terminar.</p>
      {#if readError || stale}<p class="pipeline-notice">Se conserva la última lectura disponible del pipeline.</p>{/if}
      <dl class="pipeline-lag"><div><dt>Lag p99 del temporizador</dt><dd>{stats.runtime_lag_samples ? `${display(stats.runtime_lag_p99_us / 1000)} ms` : "—"}</dd></div>
        <div><dt>Lag máximo observado</dt><dd>{stats.runtime_lag_samples ? `${display(stats.runtime_lag_max_us / 1000)} ms` : "—"}</dd></div>
        <div><dt>Muestras en la ventana</dt><dd>{display(stats.runtime_lag_window_samples)}</dd></div>
        <div><dt>Intervalo de muestreo</dt><dd>{display(stats.runtime_sample_interval_ms)} ms</dd></div></dl>
      <div class="pipeline-groups">{#each stages as stage}<section aria-label={stage.title}>
        <h3>{stage.title}</h3><dl>
          <div><dt>Trabajadores</dt><dd>{stage.value.workers ? display(stage.value.workers) : "—"}</dd></div>
          <div><dt>Trabajos reservados / límite</dt><dd>{display(stage.value.items)} / {display(stage.value.max_items)}</dd></div>
          <div><dt>Bytes reservados / límite</dt><dd>{display(stage.value.bytes)} / {display(stage.value.max_bytes)} B</dd></div>
          <div><dt>En ejecución / en cola</dt><dd>{display(stage.value.running)} / {display(stage.value.queued)}</dd></div>
          <div><dt>Pico de trabajos / bytes</dt><dd>{display(stage.value.peak_items)} / {display(stage.value.peak_bytes)} B</dd></div>
          <div><dt>Esperas por presupuesto</dt><dd>{display(stage.value.admission_waits)}</dd></div>
          <div><dt>Espera por presupuesto</dt><dd>{display(stage.value.admission_wait_us / 1000)} ms</dd></div>
          <div><dt>Espera en cola</dt><dd>{display(stage.value.queue_wait_us / 1000)} ms</dd></div>
          <div><dt>Trabajo bloqueante</dt><dd>{display(stage.value.work_us / 1000)} ms</dd></div>
          <div><dt>Trabajos completados / fallidos</dt><dd>{display(stage.value.completed)} / {display(stage.value.failed)}</dd></div>
          <div><dt>Cancelados antes de ejecutar</dt><dd>{display(stage.value.cancelled)}</dd></div>
          <div><dt>Timeouts / entradas superiores al límite</dt><dd>{display(stage.value.queue_timeouts)} / {display(stage.value.input_rejections)}</dd></div>
        </dl>
      </section>{/each}</div>
      <p>La ventana conserva las últimas 1.024 muestras del temporizador. El objetivo &lt;100 ms se verifica en el banco
        dimensionado. Batch limita cada segmento en memoria a {display(stats.batch_segment_bytes)} B y
        {display(stats.batch_segment_items)} registros; el spool conserva los datos durables.</p>
    </div>
  </details>
{/if}
<style>
  .pipeline-panel { margin-top: 24px; }
  summary { display: flex; align-items: center; flex-wrap: wrap; gap: 8px 16px; padding: 16px 24px; cursor: pointer; font-size: 12px; }
  .pipeline-summary, dt { color: #687381; }
  .pipeline-toggle { display: inline-flex; align-items: center; gap: 3px; margin-left: auto; color: #687381; }
  .pipeline-panel[open] .pipeline-toggle :global(svg) { transform: rotate(180deg); }
  .pipeline-details { padding: 0 24px 22px; font-size: 12px; line-height: 1.6; }
  .pipeline-details > p { max-width: 980px; }
  .pipeline-notice { color: #916215; }
  .pipeline-lag, .pipeline-groups { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 14px 28px; margin-top: 22px; }
  h3 { font-size: 13px; }
  dl { display: grid; gap: 12px; margin: 14px 0 0; }
  dl > div { display: flex; justify-content: space-between; gap: 16px; border-bottom: 1px solid var(--line); padding-bottom: 8px; }
  dd { margin: 0; font-weight: 600; text-align: right; }
  @media (max-width: 960px) { .pipeline-groups { grid-template-columns: 1fr; gap: 20px; } }
  @media (max-width: 620px) { summary { padding: 16px 20px; } .pipeline-details { padding: 0 20px 20px; } .pipeline-summary { order: 1; width: 100%; } .pipeline-lag { grid-template-columns: 1fr; } }
</style>
