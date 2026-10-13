<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import Icon from "./Icon.svelte";
  import type { BodyStats } from "./types";

  let { stats, readError, stale }: {
    stats?: BodyStats;
    readError: boolean;
    stale: boolean;
  } = $props();
  const number = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 1 }));
  const count = (value: number | undefined) => value == null ? "—" : number.format(value);
  const groups = $derived([
    { title: "Admisión", metrics: [
      { label: "Evaluaciones iniciadas", value: stats?.bodies_started },
      { label: "Cuerpos admitidos", value: stats?.bodies_admitted },
      { label: "Cuerpos leídos por completo", value: stats?.bodies_completed },
      { label: "Rechazos por status", value: stats?.rejected_status },
      { label: "Rechazos por MIME", value: stats?.rejected_mime },
      { label: "Rechazos por Content-Length", value: stats?.rejected_content_length },
      { label: "Codificación no admitida", value: stats?.rejected_encoding },
    ] },
    { title: "Lectura y plazos", metrics: [
      { label: "Bytes de cuerpo recibidos", value: stats?.wire_bytes_received, unit: "B" },
      { label: "Bytes de texto UTF-8 admitidos", value: stats?.decoded_bytes_produced, unit: "B" },
      { label: "Límite de recepción excedido", value: stats?.wire_limit_exceeded },
      { label: "Límite de texto excedido", value: stats?.decoded_limit_exceeded },
      { label: "Fallos al procesar el cuerpo", value: stats?.body_failures },
      { label: "Fallos del flujo de lectura", value: stats?.stream_failures },
      { label: "Fallos al reservar memoria", value: stats?.allocation_failures },
      { label: "Timeouts por inactividad", value: stats?.idle_timeouts },
      { label: "Timeouts de duración total", value: stats?.total_timeouts },
      { label: "Timeouts esperando presupuesto", value: stats?.budget_timeouts },
    ] },
    { title: "Reserva de memoria", metrics: [
      { label: "Cuerpos con reserva activa", value: stats?.active_bodies },
      { label: "Pico de cuerpos con reserva", value: stats?.peak_active_bodies },
      { label: "Límite de cuerpos simultáneos", value: stats?.max_inflight_bodies },
      { label: "Bytes reservados actuales", value: stats?.reserved_bytes, unit: "B" },
      { label: "Pico de bytes reservados", value: stats?.peak_reserved_bytes, unit: "B" },
      { label: "Límite de bytes reservados", value: stats?.max_reserved_bytes, unit: "B" },
      { label: "Esperas por presupuesto", value: stats?.budget_waits },
      { label: "Espera acumulada de presupuesto", value: stats?.budget_wait_ms, unit: "ms" },
    ] },
  ]);
</script>

{#if stats}
  <details class="panel body-panel">
    <summary>
      <span class="body-heading">{$t("Admisión y cuerpos")}</span>
      <span class="body-summary">{$t("{p0} admitidos · {p1} con reserva activa", { p0: count(stats.bodies_admitted), p1: count(stats.active_bodies) })}</span>
      <span class="body-toggle">{$t("Detalles")} <Icon name="chevron" size={16} /></span>
    </summary>
    <div class="body-details">
      <p>
        {$t("La reserva cubre los buffers del cuerpo hasta terminar su procesamiento; no representa toda la memoria del proceso. Los bytes recibidos incluyen el bloque que cruza una cuota; los bytes de texto cuentan el UTF-8 admitido. Los rechazos y fallos de lectura no son fallos al guardar WARC.")}
      </p>
      {#if readError || stale}<p class="body-notice">{$t("Se conserva la última lectura disponible de admisión y cuerpos.")}</p>{/if}
      <div class="body-groups">
        {#each groups as group}<section aria-label={$t(group.title)}>
          <h3>{$t(group.title)}</h3>
          <dl>{#each group.metrics as metric}<div>
            <dt>{$t(metric.label)}</dt><dd>{count(metric.value)}{metric.value != null && metric.unit ? ` ${metric.unit}` : ""}</dd>
          </div>{/each}</dl>
        </section>{/each}
      </div>
    </div>
  </details>
{/if}

<style>
  .body-panel { margin-top: 24px; }
  summary { display: flex; align-items: center; flex-wrap: wrap; gap: 8px 16px; padding: 16px 24px; cursor: pointer; font-size: 12px; }
  .body-heading { font-weight: 600; }
  .body-summary, dt { color: #687381; }
  .body-toggle { display: inline-flex; align-items: center; gap: 3px; margin-left: auto; color: #687381; }
  .body-panel[open] .body-toggle :global(svg) { transform: rotate(180deg); }
  .body-details { padding: 0 24px 22px; font-size: 12px; line-height: 1.6; }
  .body-details > p { max-width: 980px; }
  .body-notice { color: #916215; }
  .body-groups { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 28px; margin-top: 22px; }
  h3 { font-size: 13px; }
  dl { display: grid; gap: 12px; margin: 14px 0 0; }
  dl > div { display: flex; justify-content: space-between; gap: 16px; border-bottom: 1px solid var(--line); padding-bottom: 8px; }
  dd { margin: 0; font-weight: 600; overflow-wrap: anywhere; }
  @media (max-width: 960px) { .body-groups { grid-template-columns: 1fr; gap: 20px; } }
  @media (max-width: 620px) {
    summary { padding: 16px 20px; }
    .body-details { padding: 0 20px 20px; }
    .body-summary { order: 1; width: 100%; }
  }
</style>
