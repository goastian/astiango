<script lang="ts">
  import type { PipelineBenchmarkReport, Distribution } from "./benchmark-types";
  let { report }: { report: PipelineBenchmarkReport } = $props();
  let selected = $state("");
  const scenario = $derived(selected || report.comparisons[0]?.scenario);
  const runs = $derived(report.runs.filter((run) => run.scenario === scenario));
  const number = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 2 });
  const display = (value: number) => number.format(value);
</script>
{#snippet metric(value: Distribution, divisor = 1)}
  <strong>{display(value.median / divisor)}</strong><small>MAD {display(value.mad / divisor)}</small>
  <small>{display(value.min / divisor)}–{display(value.max / divisor)}</small>
{/snippet}
<div class="benchmark-summary">
  <span class="benchmark-result" class:passed={report.acceptance.passed}>{report.acceptance.passed ? "Aceptación verificada" : "Aceptación pendiente"}</span>
  <span>Objetivo: lag p99 &lt;100 ms</span><span>Mediana · MAD · rango · tres repeticiones</span>
</div>
<p class="benchmark-acceptance-note">Mismos cuerpos, límites y demoras; HTTP loopback, parser y WARC/fsync reales.
  La referencia ejecuta el trabajo bloqueante en Tokio. El fixture usa 1–2 hilos de runtime por escenario;
  el porcentaje de rendimiento corresponde a ese perfil. Las cuotas cuentan entradas, mientras RSS incluye DOM y buffers.</p>
<div class="benchmark-scroll"><table class="benchmark-table" aria-label="Resultados del pipeline">
  <thead><tr><th>Escenario</th><th>Modo</th><th>Orígenes permitidos</th><th>Páginas/s</th><th>CPU ms/página</th><th>Lag p99 ms</th><th>Control ms</th><th>RSS MiB</th><th>Esperas de admisión</th></tr></thead>
  <tbody>{#each report.summaries as row}<tr><th scope="row">{row.scenario}</th><td>{row.mode === "legacy" ? "Antes · inline" : "Pipeline acotado"}</td><td>{row.origins}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.pages_per_second)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.cpu_ms_per_page)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.runtime_lag_p99_ms)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.controller_delay_ms)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.rss_peak_bytes, 1048576)}</td>
    <td class="benchmark-numeric">{@render metric(row.metrics.admission_waits)}</td>
  </tr>{/each}</tbody>
</table></div>
<div class="benchmark-details-grid">
  <div class="benchmark-detail"><h3>Rendimiento y equivalencia</h3>
    <div class="benchmark-scroll"><table class="benchmark-table" aria-label="Comparación del pipeline">
      <thead><tr><th>Escenario</th><th>Cambio de páginas/s</th><th>Reducción de lag</th><th>Cuerpos WARC</th></tr></thead>
      <tbody>{#each report.comparisons as row}<tr><th scope="row">{row.scenario}</th>
        <td class="benchmark-numeric">{@render metric(row.throughput_gain_pct)} %</td>
        <td class="benchmark-numeric">{@render metric(row.lag_reduction_pct)} %</td>
        <td class:benchmark-failed={!row.equivalent}>{row.equivalent ? "Equivalentes" : "Diferencias"}</td>
      </tr>{/each}</tbody>
    </table></div>
  </div>
  <div class="benchmark-detail"><div class="benchmark-stage-heading"><h3>Saturación y repeticiones</h3>
    <label><span class="sr-only">Escenario del pipeline para inspeccionar</span><select bind:value={selected}>
      {#each report.comparisons as row}<option value={row.scenario}>{row.scenario}</option>{/each}
    </select></label></div>
    <div class="benchmark-scroll"><table class="benchmark-table" aria-label="Repeticiones del pipeline">
      <thead><tr><th>Repetición</th><th>Modo</th><th>Lag p99 ms</th><th>Pico documentos / cuota</th><th>Pico parseo / cuota bytes</th><th>HTTP / guardadas</th></tr></thead>
      <tbody>{#each runs as run}<tr><th scope="row">{run.repetition}</th><td>{run.mode === "legacy" ? "Antes" : "Acotado"}</td>
        <td>{display(run.runtime_lag_p99_ms)}</td><td>{run.pipeline.admission.peak_items} / {run.pipeline.admission.max_items}</td>
        <td>{run.pipeline.parse.peak_bytes} / {run.pipeline.parse.max_bytes}</td><td>{run.server.requests} / {run.pages}</td>
      </tr>{/each}</tbody>
    </table></div>
    <p>Disco y parser lentos producen espera por presupuesto. El control dispone de su propio pool; los trabajos bloqueantes iniciados se drenan al cerrar.</p>
  </div>
</div>
<details class="benchmark-provenance"><summary>Configuración, código y hardware</summary>
  <pre>{JSON.stringify({ configuration: report.configuration, code: report.code, binary: report.binary, hardware: report.hardware, acceptance: report.acceptance }, null, 2)}</pre>
</details>
