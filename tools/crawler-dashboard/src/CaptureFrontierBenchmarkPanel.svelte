<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import type { CaptureFrontierBenchmarkReport } from "./benchmark-types";
  let { report }: { report: CaptureFrontierBenchmarkReport } = $props();
  const formatter = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 2 }));
  const value = (v: number) => formatter.format(v);
  const mib = (v: number) => value(v / 1024 / 1024);
  const change = (reduction: number) => `${reduction >= 0 ? "−" : "+"}${value(Math.abs(reduction))} %`;
</script>
<div class="capture-frontier-benchmark">
  <p class="result">{$t("{p0} · {p1} · tres repeticiones por escenario", { p0: report.acceptance.passed ? "Aceptación verificada" : "Perfil pendiente de aceptación", p1: report.configuration.smoke ? "Prueba reducida" : "Perfil completo" })}</p>
  <p>{$t("Actualización durable de 128 URLs con el mismo corpus y 128 orígenes. La mediana se acompaña de MAD y rango. La construcción inicial de redb confirma cada lote; el baseline JSON confirma un snapshot completo.")}</p>
  <div class="table-scroll"><table aria-label={$t("Resultados de captura y frontier")}><thead><tr><th>{$t("URLs")}</th><th>{$t("Estado")}</th><th>{$t("RSS máximo · MiB")}</th><th>{$t("CPU actualización · ms")}</th><th>{$t("Actualización · ms")}</th><th>{$t("Disco · MiB")}</th></tr></thead>
    <tbody>{#each report.summaries as row}<tr><td>{value(row.urls)}</td><td>{row.mode === "redb" ? $t("redb · caché fija") : $t("JSON · snapshot completo")}</td>
      {#each ["peak_rss_bytes", "update_cpu_ms", "update_ms", "database_bytes"] as key}<td>{key.endsWith("bytes") ? mib(row.metrics[key].median) : value(row.metrics[key].median)}
        <small>{$t("MAD {p0} · {p1}–{p2}", { p0: key.endsWith("bytes") ? mib(row.metrics[key].mad) : value(row.metrics[key].mad), p1: key.endsWith("bytes") ? mib(row.metrics[key].min) : value(row.metrics[key].min), p2: key.endsWith("bytes") ? mib(row.metrics[key].max) : value(row.metrics[key].max) })}</small></td>{/each}
    </tr>{/each}</tbody></table></div>
  <ul>{#each report.comparisons as comparison}<li>{$t("{p0} URLs: RSS {p1} · CPU de actualización {p2} · tiempo de actualización {p3}", { p0: value(comparison.urls), p1: change(comparison.peak_rss_bytes_reduction_pct), p2: change(comparison.update_cpu_ms_reduction_pct), p3: change(comparison.update_ms_reduction_pct) })}</li>{/each}</ul>
  <p>{$t("{p0} comprobaciones de caída con SIGKILL · {p1} repeticiones HTTP con lector WARC independiente. El cuerpo comprimido, los redirects y las decisiones de robots se contrastan con el servidor de prueba.", { p0: report.acceptance.crash_checks, p1: report.acceptance.capture_checks })}</p>
  <p>{$t("Estos porcentajes corresponden a persistencia y memoria de la frontera. El crawl completo también depende de red, cortesía, parsing y archivo.")}</p>
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
