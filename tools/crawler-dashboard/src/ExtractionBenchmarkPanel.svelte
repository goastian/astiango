<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import type { ExtractionBenchmarkReport, Distribution } from "./benchmark-types";
  let { report }: { report: ExtractionBenchmarkReport } = $props();
  let selected = $state("");
  const scenario = $derived(selected || report.comparisons[0]?.scenario);
  const runs = $derived(report.runs.filter((run) => run.scenario === scenario));
  const number = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 3 }));
  const display = (value: number, unit = "") => `${number.format(value)}${unit ? ` ${unit}` : ""}`;
</script>
{#snippet metric(value: Distribution, unit = "")}
  <strong>{display(value.median, unit)}</strong>
  <small>{$t("MAD {p0}", { p0: display(value.mad, unit) })}</small><small>{display(value.min, unit)}–{display(value.max, unit)}</small>
{/snippet}
<div class="benchmark-summary">
  <span class="benchmark-result" class:passed={report.acceptance.passed}>{report.acceptance.passed ? $t("Aceptación verificada") : $t("Aceptación pendiente")}</span>
  <span>{$t("CPU del corpus: {p0} menos · objetivo ≥30 %", { p0: display(report.corpus_cpu_reduction_pct.median, "%") })}</span>
  <span>{$t("Mediana · MAD · rango · tres repeticiones")}</span>
</div>
<p class="benchmark-acceptance-note">{$t("Comparación del doble parseo anterior con una extracción compartida. Mismo HTML, reglas y límites. CPU medida en el hilo; bytes asignados acumulados por página, incluidos realloc, no memoria viva ni RSS. Este banco trabaja con archivos locales y no realiza solicitudes de red.")}</p>
<div class="benchmark-scroll"><table class="benchmark-table" aria-label={$t("Resultados de extracción")}>
  <thead><tr><th>{$t("Escenario")}</th><th>{$t("Modo")}</th><th>{$t("Repeticiones")}</th><th>{$t("CPU ms/página")}</th><th>{$t("Bytes asignados/página")}</th><th>{$t("Asignaciones/página")}</th><th>{$t("DOM/página")}</th></tr></thead>
  <tbody>{#each report.summaries as row}<tr>
    <th scope="row">{row.scenario}</th><td>{row.mode === "legacy" ? $t("Doble parseo") : $t("Compartida")}</td><td>{row.repetitions}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.cpu_ms_per_page)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.allocated_bytes_per_page, "B")}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.allocation_calls_per_page)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.dom_parses_per_page)}</td>
  </tr>{/each}</tbody>
</table></div>
<div class="benchmark-details-grid">
  <div class="benchmark-detail"><h3>{$t("Mejora y equivalencia")}</h3>
    <div class="benchmark-scroll"><table class="benchmark-table" aria-label={$t("Comparación de extracción")}>
      <thead><tr><th>{$t("Escenario")}</th><th>{$t("CPU reducida")}</th><th>{$t("Bytes asignados reducidos")}</th><th>{$t("Enlaces y metadatos")}</th></tr></thead>
      <tbody>{#each report.comparisons as row}<tr><th scope="row">{row.scenario}</th>
        <td class="benchmark-numeric">{@render metric(row.cpu_reduction_pct, "%")}</td>
        <td class="benchmark-numeric">{@render metric(row.allocated_bytes_reduction_pct, "%")}</td>
        <td class:benchmark-failed={!row.equivalent}>{row.equivalent ? $t("Equivalentes") : $t("Diferencias")}</td>
      </tr>{/each}</tbody>
    </table></div>
  </div>
  <div class="benchmark-detail"><div class="benchmark-stage-heading"><h3>{$t("Repeticiones y límite de enlaces")}</h3>
    <label><span class="sr-only">{$t("Escenario de extracción para inspeccionar")}</span><select bind:value={selected}>
      {#each report.comparisons as row}<option value={row.scenario}>{row.scenario}</option>{/each}
    </select></label></div>
    <div class="benchmark-scroll"><table class="benchmark-table" aria-label={$t("Repeticiones de extracción")}>
      <thead><tr><th>{$t("Repetición")}</th><th>{$t("Modo")}</th><th>{$t("CPU ms/página")}</th><th>{$t("Bytes asignados/página")}</th><th>{$t("Anchors recorridos")}</th><th>{$t("Descubrimiento / internos")}</th></tr></thead>
      <tbody>{#each runs as run}<tr><th scope="row">{run.repetition}</th><td>{run.mode === "legacy" ? $t("Doble parseo") : $t("Compartida")}</td>
        <td>{display(run.cpu_ms_per_page)}</td><td>{display(run.allocated_bytes_per_page, "B")}</td>
        <td>{run.mode === "shared" ? display(run.extraction.anchors_visited) : $t("Recorrido completo")}</td>
        <td>{run.extraction.discovery_urls.length} / {run.extraction.internal_urls.length}</td>
      </tr>{/each}</tbody>
    </table></div>
    <p>{$t("El DOM aún procesa todo el HTML. Las proyecciones retienen hasta 512 enlaces cada una; el recorrido corta al completar ambas cuotas.")}</p>
  </div>
</div>
<details class="benchmark-provenance"><summary>{$t("Configuración, código y hardware")}</summary>
  <pre>{JSON.stringify({ configuration: report.configuration, code: report.code, binary: report.binary, hardware: report.hardware, acceptance: report.acceptance }, null, 2)}</pre>
</details>
