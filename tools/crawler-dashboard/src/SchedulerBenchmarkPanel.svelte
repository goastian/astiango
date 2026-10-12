<script lang="ts">
  import type { SchedulerBenchmarkReport } from "./benchmark-types";
  let { report }: { report: SchedulerBenchmarkReport } = $props();
  const formatter = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 2 });
  const display = (value: number) => formatter.format(value);
</script>
<div class="scheduler-benchmark">
  <p class="result">{report.acceptance.passed ? "Aceptación verificada" : "Aceptación pendiente"} · C11–C13 · tres repeticiones por escenario</p>
  <p>Dos trabajadores, el mismo conjunto de páginas y la misma demora mínima. HTTP y archivo WARC reales, con todos los endpoints en loopback.</p>
  <div class="table-scroll"><table aria-label="Resultados C11 C12 C13"><thead><tr><th>Orígenes</th><th>Modelo</th><th>Páginas/s</th><th>Duración · ms</th><th>CPU · ms</th><th>RSS · MiB</th><th>Atraso p99 · ms</th></tr></thead>
    <tbody>{#each report.summaries as row}<tr><td>{display(row.origins)}</td><td>{row.mode === "fair_turns" ? "Turnos cortos" : "Jobs largos"}</td>
      {#each ["pages_per_second", "duration_ms", "cpu_ms", "rss_peak_bytes", "ready_lag_p99_ms"] as key}<td>{display(row.metrics[key].median / (key === "rss_peak_bytes" ? 1048576 : 1))}
        <small>MAD {display(row.metrics[key].mad / (key === "rss_peak_bytes" ? 1048576 : 1))}</small></td>{/each}
    </tr>{/each}</tbody></table></div>
  <ul>{#each report.comparisons as row}<li>{display(row.origins)} orígenes: {display(row.speedup)}× páginas/s · mejora {display(row.throughput_gain_pct)} %.</li>{/each}</ul>
  <p>{display(report.acceptance.courtesy_violations)} violaciones de cortesía. Contadores y conjuntos de peticiones contrastados con el servidor.
    Estas ganancias corresponden al fixture dimensionado; Internet, almacenamiento y pocos orígenes pueden limitar el rendimiento.</p>
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
