<script lang="ts">
  import type { ResilienceBenchmarkReport } from "./benchmark-types";
  let { report }: { report: ResilienceBenchmarkReport } = $props();
  const format = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 1 });
  const number = (n: number) => format.format(n);
  const diversity = $derived(report.scenarios.find(s => s.name === "diversity")?.runs[0]?.comparisons as Array<{ tenants: number; urls: number }> | undefined);
</script>
<section class="resilience-benchmark" aria-label="Banco C14–C20">
  <h3>Revalidación, descubrimiento y planificación</h3>
  <p>Tres repeticiones por escenario. Mediana, rango y desviación absoluta mediana (MAD). El tráfico de prueba permanece en loopback.</p>
  <div class="comparison"><strong>{number(report.comparisons.body_bytes)} %</strong><span>menos bytes HTTP en la cohorte local de dos rondas, con 80 % de páginas sin cambios.</span></div>
  <div class="table-scroll"><table><thead><tr><th>Escenario</th><th>URLs</th><th>Tiempo · ms</th><th>CPU · ms</th><th>RSS · MiB</th></tr></thead><tbody>
    {#each report.scenarios as scenario}<tr>
      <td>{scenario.name === "recrawl" ? (scenario.mode === "conditional" ? "GET condicional" : "GET completo") : scenario.name === "planner" ? "Plan en disco" : scenario.name === "traps" ? (scenario.mode === "protected" ? "Trampas · cuotas activas" : "Trampas · referencia") : scenario.name === "diversity" ? "Diversidad · igual presupuesto" : "Sitemap incremental"}</td>
      <td>{number(scenario.urls || report.configuration.pages)}</td>
      {#each [scenario.metrics.duration_ms, scenario.metrics.cpu_ms, scenario.metrics.rss_after_bytes] as value, index}<td>{number(value.median / (index === 2 ? 1048576 : 1))}<small>{number(value.min / (index === 2 ? 1048576 : 1))}–{number(value.max / (index === 2 ? 1048576 : 1))} · MAD {number(value.mad / (index === 2 ? 1048576 : 1))}</small></td>{/each}
    </tr>{/each}
  </tbody></table></div>
  <p>La comparación de CPU y tiempo pertenece a esta cohorte: {number(report.comparisons.cpu_ms)} % y {number(report.comparisons.duration_ms)} % de reducción. No representa una ganancia global del crawler.</p>
  {#if report.comparisons.trap_requests !== undefined}<p>Trampas: {number(report.comparisons.trap_requests)} % menos solicitudes en el corpus controlado, conservando las 200 páginas legítimas.</p>{/if}
  {#if diversity?.length === 2}<p>Diversidad a igual presupuesto de {number(diversity[0].urls)} URLs: {number(diversity[0].tenants)} → {number(diversity[1].tenants)} sitios representados al reservar turnos para exploración.</p>{/if}
  <p>Recuperación del planner: {report.recovery.length} cierres forzados, con 100.000 candidatos conservados en cada recuperación. Caché: {number(report.configuration.planner_cache_bytes / 1048576)} MiB.</p>
  <p class="status">Aceptación del banco: {report.acceptance.passed ? "verificada" : "pendiente"} · Código {report.code.commit.slice(0, 12)} · {report.code.source_sha256.slice(0, 12)}</p>
</section>
<style>
  h3 { font-size: 14px; margin: 24px 0 12px; } p { font-size: 12px; color: #697386; line-height: 1.6; }
  .comparison { display:flex; align-items:center; gap:24px; padding:20px; background:#f6f8fb; border:1px solid #e7eaf0; border-radius:4px; margin:20px 0; }
  .comparison strong { font-size:28px; font-weight:500; white-space:nowrap; } .comparison span { font-size:13px; max-width:650px; }
  .table-scroll { overflow-x:auto; } table { border-collapse:collapse; width:100%; font-size:12px; } th,td { padding:12px; border-bottom:1px solid #e7eaf0; text-align:left; } th { font-weight:600; } small { display:block; margin-top:6px; color:#697386; font-size:11px; } .status { margin-top:20px; }
</style>
