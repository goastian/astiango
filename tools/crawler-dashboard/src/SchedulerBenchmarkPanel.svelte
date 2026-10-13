<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import type { SchedulerBenchmarkReport } from "./benchmark-types";
  let { report }: { report: SchedulerBenchmarkReport } = $props();
  const formatter = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 2 }));
  const display = (value: number) => formatter.format(value);
</script>
<div class="scheduler-benchmark">
  <p class="result">{$t("{p0} · identidad, cortesía y robots · tres repeticiones por escenario", { p0: report.acceptance.passed ? "Aceptación verificada" : "Aceptación pendiente" })}</p>
  <p>{$t("Dos trabajadores, el mismo conjunto de páginas y la misma demora mínima. HTTP y archivo WARC reales, con todos los endpoints en loopback.")}</p>
  <div class="table-scroll"><table aria-label={$t("Resultados de identidad, cortesía y robots")}><thead><tr><th>{$t("Orígenes")}</th><th>{$t("Modelo")}</th><th>{$t("Páginas/s")}</th><th>{$t("Duración · ms")}</th><th>{$t("CPU · ms")}</th><th>{$t("RSS · MiB")}</th><th>{$t("Atraso p99 · ms")}</th></tr></thead>
    <tbody>{#each report.summaries as row}<tr><td>{display(row.origins)}</td><td>{row.mode === "fair_turns" ? $t("Turnos cortos") : $t("Jobs largos")}</td>
      {#each ["pages_per_second", "duration_ms", "cpu_ms", "rss_peak_bytes", "ready_lag_p99_ms"] as key}<td>{display(row.metrics[key].median / (key === "rss_peak_bytes" ? 1048576 : 1))}
        <small>{$t("MAD {p0}", { p0: display(row.metrics[key].mad / (key === "rss_peak_bytes" ? 1048576 : 1)) })}</small></td>{/each}
    </tr>{/each}</tbody></table></div>
  <ul>{#each report.comparisons as row}<li>{$t("{p0} orígenes: {p1}× páginas/s · mejora {p2} %.", { p0: display(row.origins), p1: display(row.speedup), p2: display(row.throughput_gain_pct) })}</li>{/each}</ul>
  <p>{$t("{p0} violaciones de cortesía. Contadores y conjuntos de peticiones contrastados con el servidor. Estas ganancias corresponden al fixture dimensionado; Internet, almacenamiento y pocos orígenes pueden limitar el rendimiento.", { p0: display(report.acceptance.courtesy_violations) })}</p>
</div>
<style>
  p, li { font-size: 12px; line-height: 1.6; color: #697386; }
  .result { color: #1d7652; font-weight: 600; }
  .table-scroll { overflow-x: auto; }
  table { border-collapse: collapse; width: 100%; font-size: 12px; }
  th, td { padding: 12px; border-bottom: 1px solid #e9edf3; text-align: left; }
  th { white-space: nowrap; }
  small { display: block; font-size: 10px; color: #8490a1; margin-top: 5px; }
  ul { padding-left: 18px; }
</style>
