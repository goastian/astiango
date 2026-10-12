<script lang="ts">
  import type { Snapshot } from "./types";
  import { observerHealth, snapshotAgeMs } from "./metrics";
  import Icon from "./Icon.svelte";

  let { snapshot, now, readError, stale, paused }: {
    snapshot?: Snapshot;
    now: number;
    readError: boolean;
    stale: boolean;
    paused: boolean;
  } = $props();
  const stats = $derived(snapshot?.telemetry);
  const age = $derived(snapshotAgeMs(snapshot, now));
  const health = $derived(observerHealth(stats, { readError, stale, paused }));
  const number = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 1 });
  const count = (value: number | undefined) => value == null ? "—" : number.format(value);
  const seconds = (value: number | null | undefined) => value == null ? "—" : `${number.format(value / 1000)} s`;
  const latency = (value: number | undefined) => value == null ? "—" : value < 1000 ? `${count(value)} ms` : seconds(value);
  const date = (value: string | null | undefined) => value
    ? new Date(value).toLocaleString("es-CO", { timeZone: "America/Bogota" })
    : "—";
</script>

{#if stats}
  <details class="panel telemetry-panel" class:degraded={health.warning}>
    <summary>
      <span class="telemetry-heading">Observador</span>
      <span class="telemetry-health">{health.label}</span>
      <span class="telemetry-age">Publicado hace {seconds(age)} · agrupación cada {seconds(stats.projection_interval_ms)}</span>
      <span class="telemetry-toggle">Detalles <Icon name="chevron" size={16} /></span>
    </summary>
    <div class="telemetry-details">
      <p>
        Los cambios del panel se agrupan antes de escribir el estado. Los eventos
        visuales tienen un historial limitado; sus omisiones no descuentan los
        totales del rastreo ni representan fallos al guardar páginas.
      </p>
      <dl>
        <div><dt>Última publicación</dt><dd>{date(stats.last_published_at)}</dd></div>
        <div><dt>Actualizaciones recibidas</dt><dd>{count(stats.updates)}</dd></div>
        <div><dt>Publicaciones atómicas</dt><dd>{count(stats.snapshot_writes)}</dd></div>
        <div><dt>Llamadas de escritura</dt><dd>{count(stats.write_calls)}</dd></div>
        <div><dt>Bytes escritos por el observador</dt><dd>{count(stats.bytes_written)} B</dd></div>
        <div><dt>Actualizaciones pendientes</dt><dd>{count(stats.pending_updates)}</dd></div>
        <div><dt>Demora de última publicación</dt><dd>{latency(stats.last_publish_latency_ms)}</dd></div>
        <div><dt>Demora máxima de publicación</dt><dd>{latency(stats.max_publish_latency_ms)}</dd></div>
        <div><dt>Eventos recibidos</dt><dd>{count(stats.events_total)}</dd></div>
        <div><dt>Eventos fuera del buffer</dt><dd>{count(stats.events_dropped)}</dd></div>
        <div><dt>Capacidad del historial</dt><dd>{count(stats.event_capacity)}</dd></div>
        <div><dt>Errores de publicación</dt><dd>{count(stats.errors)}</dd></div>
        <div><dt>Último error de publicación</dt><dd>{date(stats.last_failure_at)}</dd></div>
      </dl>
      {#if stats.last_error}
        <p class="telemetry-error"><strong>Error del observador:</strong> {stats.last_error}</p>
      {:else if stats.errors > 0}
        <p>La última publicación no registra un error activo; se conserva el total de errores anteriores.</p>
      {/if}
      {#if stale || readError}
        <p class="telemetry-error">Se conserva la última lectura disponible. El fallo de la salida de métricas no demuestra un fallo del crawler o de los WARC.</p>
      {/if}
    </div>
  </details>
{/if}
