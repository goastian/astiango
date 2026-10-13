<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import Icon from "./Icon.svelte";
  import type { SpoolStats } from "./types";
  let { stats, readError, stale }: { stats?: SpoolStats; readError: boolean; stale: boolean } = $props();
  const number = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 2 }));
  const display = (value: number) => number.format(value);
  const mib = (value: number) => display(value / 1024 / 1024) + " MiB";
  const groups = $derived(stats ? [
    { title: "Archivo local durable", values: [
      ["Páginas durables en el spool", display(stats.records_durable)],
      ["Páginas recuperadas", display(stats.records_recovered)],
      ["Segmentos / límite", display(stats.segments_total) + " / " + display(stats.max_segments)],
      ["Segmentos abiertos", display(stats.segments_open)],
      ["Espacio del spool / cuota", mib(stats.spool_bytes) + " / " + mib(stats.max_spool_bytes)],
      ["Rotación por tamaño", mib(stats.segment_bytes)],
      ["Nivel gzip", display(stats.gzip_level)],
      ["Grupos confirmados", display(stats.group_commits)],
      ["Sincronizaciones", display(stats.sync_calls)],
      ["Rechazos por cuota", display(stats.quota_rejections)],
      ["Errores locales de almacenamiento", display(stats.errors)],
    ]},
    { title: "Publicación y retención", values: [
      ["Segmentos pendientes de S3", display(stats.segments_pending)],
      ["Segmentos publicados", display(stats.segments_published)],
      ["Intentos de publicación", display(stats.upload_attempts)],
      ["Errores de publicación", display(stats.upload_errors)],
      ["Buffer de upload / pico", mib(stats.upload_buffer_bytes) + " / " + mib(stats.upload_buffer_peak_bytes)],
      ["Referencias que conservan segmentos", display(stats.reference_pins)],
      ["Intentos de GC bloqueados", display(stats.gc_blocked)],
      ["Segmentos eliminados por GC", display(stats.gc_deleted)],
    ]},
  ] : []);
</script>
{#if stats}
  <details class="panel spool-panel">
    <summary><strong>{$t("Spool durable y segmentos")}</strong>
      <span class="spool-summary">{$t("{p0} páginas durables · {p1} pendientes de S3", { p0: display(stats.records_durable), p1: display(stats.segments_pending) })}</span>
      <span class="spool-toggle">{$t("Detalles")} <Icon name="chevron" size={16} /></span>
    </summary>
    <div class="spool-details">
      <p>{$t("El ACK confirma datos y registro de recuperación sincronizados en disco local. La publicación S3 tiene su propia cola durable. Estos totales incluyen los segmentos recuperados; las páginas de esta ejecución permanecen en el resumen superior.")}</p>
      {#if readError || stale}<p class="spool-notice">{$t("Se conserva la última lectura disponible del spool.")}</p>{/if}
      {#if stats.last_error}<p class="spool-notice">{$t("Última incidencia: {p0}", { p0: stats.last_error })}</p>{/if}
      <div class="spool-groups">{#each groups as group}<section aria-label={$t(group.title)}><h3>{$t(group.title)}</h3>
        <dl>{#each group.values as [label, value]}<div><dt>{$t(label)}</dt><dd>{$t(value)}</dd></div>{/each}</dl>
      </section>{/each}</div>
      <p>{$t("Una cuota llena detiene la admisión. Las referencias de revisitas, 304, consumidores, outbox y backups impiden borrar segmentos necesarios. El buffer indicado pertenece al uploader; no representa todo el RSS.")}</p>
    </div>
  </details>
{/if}
<style>
  .spool-panel { margin-top: 20px; }
  summary { display: flex; gap: 16px; align-items: center; padding: 20px 24px; cursor: pointer; list-style: none; }
  summary::-webkit-details-marker { display: none; }
  summary strong, h3 { font-size: 12px; }
  .spool-summary, .spool-toggle { color: #7d8799; font-size: 11px; }
  .spool-toggle { margin-left: auto; display: flex; align-items: center; gap: 8px; }
  .spool-details { padding: 0 24px 20px; }
  p { color: #7d8799; font-size: 11px; line-height: 1.65; }
  .spool-groups { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 24px; }
  h3 { margin: 16px 0; }
  dl { margin: 0; font-size: 11px; }
  dl > div { display: flex; justify-content: space-between; gap: 16px; padding: 9px 0; border-bottom: 1px solid #eef0f5; }
  dt { color: #7d8799; } dd { margin: 0; text-align: right; font-variant-numeric: tabular-nums; }
  .spool-notice { padding: 12px; background: #fff8e9; color: #91610d; overflow-wrap: anywhere; }
  @media (max-width: 720px) { summary { flex-wrap: wrap; padding: 16px; } .spool-summary { width: 100%; order: 3; } .spool-details { padding: 0 16px 16px; } .spool-groups { grid-template-columns: 1fr; gap: 8px; } }
</style>
