<script lang="ts">
  import type { ExtractionBenchmarkReport, Distribution } from "./benchmark-types";
  let { report }: { report: ExtractionBenchmarkReport } = $props();
  let selected = $state("");
  const scenario = $derived(selected || report.comparisons[0]?.scenario);
  const runs = $derived(report.runs.filter((run) => run.scenario === scenario));
  const number = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 3 });
  const display = (value: number, unit = "") => `${number.format(value)}${unit ? ` ${unit}` : ""}`;
</script>
{#snippet metric(value: Distribution, unit = "")}
  <strong>{display(value.median, unit)}</strong>
  <small>MAD {display(value.mad, unit)}</small><small>{display(value.min, unit)}–{display(value.max, unit)}</small>
{/snippet}
<div class="benchmark-summary">
  <span class="benchmark-result" class:passed={report.acceptance.passed}>{report.acceptance.passed ? "Aceptación verificada" : "Aceptación pendiente"}</span>
  <span>CPU del corpus: {display(report.corpus_cpu_reduction_pct.median, "%")} menos · objetivo ≥30 %</span>
  <span>Mediana · MAD · rango · tres repeticiones</span>
</div>
<p class="benchmark-acceptance-note">Comparación del doble parseo de C05 con una extracción compartida.
  Mismo HTML, reglas y límites. CPU medida en el hilo; bytes asignados acumulados por página, incluidos realloc,
  no memoria viva ni RSS. Este banco trabaja con archivos locales y no realiza solicitudes de red.</p>
<div class="benchmark-scroll"><table class="benchmark-table" aria-label="Resultados de extracción C06">
  <thead><tr><th>Escenario</th><th>Modo</th><th>Repeticiones</th><th>CPU ms/página</th><th>Bytes asignados/página</th><th>Asignaciones/página</th><th>DOM/página</th></tr></thead>
  <tbody>{#each report.summaries as row}<tr>
    <th scope="row">{row.scenario}</th><td>{row.mode === "legacy" ? "Doble parseo" : "Compartida"}</td><td>{row.repetitions}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.cpu_ms_per_page)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.allocated_bytes_per_page, "B")}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.allocation_calls_per_page)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.dom_parses_per_page)}</td>
  </tr>{/each}</tbody>
</table></div>
<div class="benchmark-details-grid">
  <div class="benchmark-detail"><h3>Mejora y equivalencia</h3>
    <div class="benchmark-scroll"><table class="benchmark-table" aria-label="Comparación de extracción C06">
      <thead><tr><th>Escenario</th><th>CPU reducida</th><th>Bytes asignados reducidos</th><th>Enlaces y metadatos</th></tr></thead>
      <tbody>{#each report.comparisons as row}<tr><th scope="row">{row.scenario}</th>
        <td class="benchmark-numeric">{@render metric(row.cpu_reduction_pct, "%")}</td>
        <td class="benchmark-numeric">{@render metric(row.allocated_bytes_reduction_pct, "%")}</td>
        <td class:benchmark-failed={!row.equivalent}>{row.equivalent ? "Equivalentes" : "Diferencias"}</td>
      </tr>{/each}</tbody>
    </table></div>
  </div>
  <div class="benchmark-detail"><div class="benchmark-stage-heading"><h3>Repeticiones y límite de enlaces</h3>
    <label><span class="sr-only">Escenario de extracción para inspeccionar</span><select bind:value={selected}>
      {#each report.comparisons as row}<option value={row.scenario}>{row.scenario}</option>{/each}
    </select></label></div>
    <div class="benchmark-scroll"><table class="benchmark-table" aria-label="Repeticiones de extracción C06">
      <thead><tr><th>Repetición</th><th>Modo</th><th>CPU ms/página</th><th>Bytes asignados/página</th><th>Anchors recorridos</th><th>Descubrimiento / internos</th></tr></thead>
      <tbody>{#each runs as run}<tr><th scope="row">{run.repetition}</th><td>{run.mode === "legacy" ? "Doble parseo" : "Compartida"}</td>
        <td>{display(run.cpu_ms_per_page)}</td><td>{display(run.allocated_bytes_per_page, "B")}</td>
        <td>{run.mode === "shared" ? display(run.extraction.anchors_visited) : "Recorrido completo"}</td>
        <td>{run.extraction.discovery_urls.length} / {run.extraction.internal_urls.length}</td>
      </tr>{/each}</tbody>
    </table></div>
    <p>El DOM aún procesa todo el HTML. Las proyecciones retienen hasta 512 enlaces cada una; el recorrido corta al completar ambas cuotas.</p>
  </div>
</div>
<details class="benchmark-provenance"><summary>Configuración, código y hardware</summary>
  <pre>{JSON.stringify({ configuration: report.configuration, code: report.code, binary: report.binary, hardware: report.hardware, acceptance: report.acceptance }, null, 2)}</pre>
</details>
