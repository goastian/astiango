<script lang="ts">
  import type { CaptureFrontierBenchmarkReport } from "./benchmark-types";
  let { report }: { report: CaptureFrontierBenchmarkReport } = $props();
  const formatter = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 2 });
  const value = (v: number) => formatter.format(v);
  const mib = (v: number) => value(v / 1024 / 1024);
  const change = (reduction: number) => `${reduction >= 0 ? "−" : "+"}${value(Math.abs(reduction))} %`;
</script>
<div class="capture-frontier-benchmark">
  <p class="result">{report.acceptance.passed ? "Aceptación verificada" : "Perfil pendiente de aceptación"} · {report.configuration.smoke ? "Prueba reducida" : "Perfil completo"} · tres repeticiones por escenario</p>
  <p>Actualización durable de 128 URLs con el mismo corpus y 128 orígenes. La mediana se acompaña de MAD y rango.
    La construcción inicial de redb confirma cada lote; el baseline JSON confirma un snapshot completo.</p>
  <div class="table-scroll"><table aria-label="Resultados de captura y frontier"><thead><tr><th>URLs</th><th>Estado</th><th>RSS máximo · MiB</th><th>CPU actualización · ms</th><th>Actualización · ms</th><th>Disco · MiB</th></tr></thead>
    <tbody>{#each report.summaries as row}<tr><td>{value(row.urls)}</td><td>{row.mode === "redb" ? "redb · caché fija" : "JSON · snapshot completo"}</td>
      {#each ["peak_rss_bytes", "update_cpu_ms", "update_ms", "database_bytes"] as key}<td>{key.endsWith("bytes") ? mib(row.metrics[key].median) : value(row.metrics[key].median)}
        <small>MAD {key.endsWith("bytes") ? mib(row.metrics[key].mad) : value(row.metrics[key].mad)} · {key.endsWith("bytes") ? mib(row.metrics[key].min) : value(row.metrics[key].min)}–{key.endsWith("bytes") ? mib(row.metrics[key].max) : value(row.metrics[key].max)}</small></td>{/each}
    </tr>{/each}</tbody></table></div>
  <ul>{#each report.comparisons as comparison}<li>{value(comparison.urls)} URLs: RSS {change(comparison.peak_rss_bytes_reduction_pct)} · CPU de actualización {change(comparison.update_cpu_ms_reduction_pct)} · tiempo de actualización {change(comparison.update_ms_reduction_pct)}</li>{/each}</ul>
  <p>{report.acceptance.crash_checks} comprobaciones de caída con SIGKILL · {report.acceptance.capture_checks} repeticiones HTTP con lector WARC independiente.
    El cuerpo comprimido, los redirects y las decisiones de robots se contrastan con el servidor de prueba.</p>
  <p>Estos porcentajes corresponden a persistencia y memoria de la frontera. El crawl completo también depende de red, cortesía, parsing y archivo.</p>
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
