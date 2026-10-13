<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import type { Distribution, SpoolBenchmarkReport } from "./benchmark-types";
  let { report }: { report: SpoolBenchmarkReport } = $props();
  const number = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 4 }));
  const display = (value: number) => number.format(value);
  const distribution = (value: Distribution) => "MAD " + display(value.mad) + " · " + display(value.min) + "–" + display(value.max);
  let selected = $state("corpus_group_16");
  const scenarios = $derived([...new Set(report.runs.map((row) => row.scenario))]);
  const rows = $derived(report.runs.filter((row) => row.scenario === selected));
  const checks = $derived([...new Set(report.checks.map((row) => row.name))].map((name) => ({
    name, passed: report.checks.filter((row) => row.name === name).every((row) => row.passed),
    repetitions: report.checks.filter((row) => row.name === name).length,
  })));
  const mode = (value: string) => value === "segments" ? "Segmentos durables" : "Anterior por página";
</script>
<div class="spool-benchmark">
  <p class="spool-result">{$t("{p0} · {p1} · Mediana, MAD y rango · 3 repeticiones", { p0: report.acceptance.passed ? "Aceptación verificada" : "Aceptación pendiente", p1: report.acceptance.full_profile ? "Perfil completo" : "Muestra reducida" })}</p>
  <p>{$t("El corpus real conservado permite comparar escritura y compresión. Los porcentajes pertenecen a esta etapa y a estos perfiles; la cortesía y la red del crawl real tienen sus propios límites. Los casos adversariales verifican rotación y memoria al aumentar el archivo.")}</p>
  <div class="spool-table"><table aria-label={$t("Resultados del archivo durable")}><thead><tr>
    <th>{$t("Escenario")}</th><th>{$t("Modo")}</th><th>{$t("Gzip / segmento")}</th><th>{$t("Páginas/s")}</th><th>{$t("CPU-ms/página")}</th>
    <th>{$t("Bytes/página")}</th><th>{$t("Sync/página")}</th><th>{$t("WARC/página")}</th><th>{$t("RSS · MiB")}</th>
  </tr></thead><tbody>{#each report.summaries as row}<tr>
    <th scope="row">{row.scenario}</th><td>{$t(mode(row.mode))}</td><td>{row.gzip_level} / {display(row.segment_bytes / 1048576)} MiB</td>
    {#each ["pages_per_second", "cpu_ms_per_page", "compressed_bytes_per_page", "sync_calls_per_page", "files_per_page", "rss_peak_bytes"] as key}
      <td><strong>{display(row.metrics[key].median / (key === "rss_peak_bytes" ? 1048576 : 1))}</strong>
        <small>{distribution(row.metrics[key])}{key === "rss_peak_bytes" ? $t("bytes") : ""}</small></td>
    {/each}
  </tr>{/each}</tbody></table></div>
  <h3>{$t("Comparación con la escritura anterior")}</h3>
  <div class="spool-table"><table aria-label={$t("Mejoras de almacenamiento")}><thead><tr>
    <th>{$t("Escenario")}</th><th>{$t("Nivel gzip")}</th><th>{$t("Variación de páginas/s")}</th><th>{$t("Reducción de sync")}</th><th>{$t("Reducción de archivos WARC")}</th>
  </tr></thead><tbody>{#each report.comparisons as row}<tr><th scope="row">{row.scenario}</th><td>{row.gzip_level}</td>
    {#each [row.throughput_gain_pct, row.sync_reduction_pct, row.files_reduction_pct] as value}
      <td><strong>{display(value.median)} %</strong><small>{distribution(value)}</small></td>
    {/each}</tr>{/each}</tbody></table></div>
  <h3>{$t("Recuperación y publicación controlada")}</h3>
  <div class="spool-table"><table aria-label={$t("Matriz de recuperación")}><thead><tr><th>{$t("Caso")}</th><th>{$t("Repeticiones")}</th><th>{$t("Resultado")}</th></tr></thead>
    <tbody>{#each checks as row}<tr><th scope="row">{$t(row.name)}</th><td>{row.repetitions}</td><td>{row.passed ? $t("Verificado") : $t("Pendiente")}</td></tr>{/each}</tbody>
  </table></div>
  <label>{$t("Escenario para inspeccionar")} <select aria-label={$t("Escenario del spool para inspeccionar")} bind:value={selected}>
    {#each scenarios as scenario}<option value={scenario}>{scenario}</option>{/each}</select></label>
  <div class="spool-table"><table aria-label={$t("Repeticiones del archivo durable")}><thead><tr><th>{$t("Modo")}</th><th>{$t("Gzip")}</th><th>{$t("Rep.")}</th><th>{$t("Páginas")}</th><th>{$t("Páginas/s")}</th><th>{$t("Sync")}</th><th>{$t("Archivos")}</th><th>{$t("RSS · MiB")}</th></tr></thead>
    <tbody>{#each rows as row}<tr><th scope="row">{$t(mode(row.mode))}</th><td>{row.gzip_level}</td><td>{row.repetition}</td><td>{row.pages}</td><td>{display(row.pages_per_second)}</td>
      <td>{row.sync_calls}</td><td>{row.files}</td><td>{display(row.rss_peak_bytes / 1048576)}</td></tr>{/each}</tbody></table></div>
  <p>{$t("El ACK es local y durable; un upload interrumpido conserva el segmento y su outbox. La descarga del informe contiene checksums, rangos confirmados, configuración, hardware, versión de código y resultados de cada repetición.")}</p>
</div>
<style>
  p { font-size: 12px; line-height: 1.65; color: #7d8799; }
  .spool-result { color: #258b57; }
  h3 { margin-top: 24px; font-size: 12px; }
  .spool-table { max-width: 100%; overflow-x: auto; margin: 14px 0; }
  table { width: 100%; border-collapse: collapse; font-size: 11px; }
  th, td { text-align: left; padding: 10px; border-bottom: 1px solid #edf0f5; vertical-align: top; }
  thead { background: #f5f7fa; color: #788296; }
  small { display: block; font-size: 9px; color: #7d8799; min-width: 86px; margin-top: 5px; }
  label { display: flex; gap: 12px; align-items: center; font-size: 12px; margin-top: 20px; }
  select { max-width: 100%; padding: 9px; border: 1px solid #d5dbea; border-radius: 3px; background: white; }
  @media (max-width: 600px) { label { align-items: stretch; flex-direction: column; } }
</style>
