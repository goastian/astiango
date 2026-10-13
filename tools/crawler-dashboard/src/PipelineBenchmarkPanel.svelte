<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import type { PipelineBenchmarkReport, Distribution } from "./benchmark-types";
  let { report }: { report: PipelineBenchmarkReport } = $props();
  let selected = $state("");
  const scenario = $derived(selected || report.comparisons[0]?.scenario);
  const runs = $derived(report.runs.filter((run) => run.scenario === scenario));
  const number = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 2 }));
  const display = (value: number) => number.format(value);
</script>
{#snippet metric(value: Distribution, divisor = 1)}
  <strong>{display(value.median / divisor)}</strong><small>{$t("MAD {p0}", { p0: display(value.mad / divisor) })}</small>
  <small>{display(value.min / divisor)}–{display(value.max / divisor)}</small>
{/snippet}
<div class="benchmark-summary">
  <span class="benchmark-result" class:passed={report.acceptance.passed}>{report.acceptance.passed ? $t("Aceptación verificada") : $t("Aceptación pendiente")}</span>
  <span>{$t("Objetivo: lag p99 <100 ms")}</span><span>{$t("Mediana · MAD · rango · tres repeticiones")}</span>
</div>
<p class="benchmark-acceptance-note">{$t("Mismos cuerpos, límites y demoras; HTTP loopback, parser y WARC/fsync reales. La referencia ejecuta el trabajo bloqueante en Tokio. El fixture usa 1–2 hilos de runtime por escenario; el porcentaje de rendimiento corresponde a ese perfil. Las cuotas cuentan entradas, mientras RSS incluye DOM y buffers.")}</p>
<div class="benchmark-scroll"><table class="benchmark-table" aria-label={$t("Resultados del pipeline")}>
  <thead><tr><th>{$t("Escenario")}</th><th>{$t("Modo")}</th><th>{$t("Orígenes permitidos")}</th><th>{$t("Páginas/s")}</th><th>{$t("CPU ms/página")}</th><th>{$t("Lag p99 ms")}</th><th>{$t("Control ms")}</th><th>{$t("RSS MiB")}</th><th>{$t("Esperas de admisión")}</th></tr></thead>
  <tbody>{#each report.summaries as row}<tr><th scope="row">{row.scenario}</th><td>{row.mode === "legacy" ? $t("Antes · inline") : $t("Pipeline acotado")}</td><td>{row.origins}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.pages_per_second)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.cpu_ms_per_page)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.runtime_lag_p99_ms)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.controller_delay_ms)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.rss_peak_bytes, 1048576)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.admission_waits)}</td>
  </tr>{/each}</tbody>
</table></div>
<div class="benchmark-details-grid">
  <div class="benchmark-detail"><h3>{$t("Rendimiento y equivalencia")}</h3>
    <div class="benchmark-scroll"><table class="benchmark-table" aria-label={$t("Comparación del pipeline")}>
      <thead><tr><th>{$t("Escenario")}</th><th>{$t("Cambio de páginas/s")}</th><th>{$t("Reducción de lag")}</th><th>{$t("Cuerpos WARC")}</th></tr></thead>
      <tbody>{#each report.comparisons as row}<tr><th scope="row">{row.scenario}</th>
        <td class="benchmark-numeric">{@render metric(row.throughput_gain_pct)} %</td>
        <td class="benchmark-numeric">{@render metric(row.lag_reduction_pct)} %</td>
        <td class:benchmark-failed={!row.equivalent}>{row.equivalent ? $t("Equivalentes") : $t("Diferencias")}</td>
      </tr>{/each}</tbody>
    </table></div>
  </div>
  <div class="benchmark-detail"><div class="benchmark-stage-heading"><h3>{$t("Saturación y repeticiones")}</h3>
    <label><span class="sr-only">{$t("Escenario del pipeline para inspeccionar")}</span><select bind:value={selected}>
      {#each report.comparisons as row}<option value={row.scenario}>{row.scenario}</option>{/each}
    </select></label></div>
    <div class="benchmark-scroll"><table class="benchmark-table" aria-label={$t("Repeticiones del pipeline")}>
      <thead><tr><th>{$t("Repetición")}</th><th>{$t("Modo")}</th><th>{$t("Lag p99 ms")}</th><th>{$t("Pico documentos / cuota")}</th><th>{$t("Pico parseo / cuota bytes")}</th><th>{$t("HTTP / guardadas")}</th></tr></thead>
      <tbody>{#each runs as run}<tr><th scope="row">{run.repetition}</th><td>{run.mode === "legacy" ? $t("Antes") : $t("Acotado")}</td>
        <td>{display(run.runtime_lag_p99_ms)}</td><td>{run.pipeline.admission.peak_items} / {run.pipeline.admission.max_items}</td>
        <td>{run.pipeline.parse.peak_bytes} / {run.pipeline.parse.max_bytes}</td><td>{run.server.requests} / {run.pages}</td>
      </tr>{/each}</tbody>
    </table></div>
    <p>{$t("Disco y parser lentos producen espera por presupuesto. El control dispone de su propio pool; los trabajos bloqueantes iniciados se drenan al cerrar.")}</p>
  </div>
</div>
<details class="benchmark-provenance"><summary>{$t("Configuración, código y hardware")}</summary>
  <pre>{JSON.stringify({ configuration: report.configuration, code: report.code, binary: report.binary, hardware: report.hardware, acceptance: report.acceptance }, null, 2)}</pre>
</details>
