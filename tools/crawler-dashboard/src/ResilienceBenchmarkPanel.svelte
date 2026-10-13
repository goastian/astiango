<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import type { ResilienceBenchmarkReport } from "./benchmark-types";
  let { report }: { report: ResilienceBenchmarkReport } = $props();
  const format = new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 1 });
  const number = (n: number) => format.format(n);
  const diversity = $derived(report.scenarios.find(s => s.name === "diversity")?.runs[0]?.comparisons as Array<{ tenants: number; urls: number }> | undefined);
</script>
<section class="resilience-benchmark" aria-label={$t("Resultados de revalidación y planificación")}>
  <h3>{$t("Revalidación, descubrimiento y planificación")}</h3>
  <p>{$t("Tres repeticiones por escenario. Mediana, rango y desviación absoluta mediana (MAD). El tráfico de prueba permanece en loopback.")}</p>
  <div class="comparison"><strong>{number(report.comparisons.body_bytes)} %</strong><span>{$t("menos bytes HTTP en la cohorte local de dos rondas, con 80 % de páginas sin cambios.")}</span></div>
  <div class="table-scroll"><table><thead><tr><th>{$t("Escenario")}</th><th>{$t("URLs")}</th><th>{$t("Tiempo · ms")}</th><th>{$t("CPU · ms")}</th><th>{$t("RSS · MiB")}</th></tr></thead><tbody>
    {#each report.scenarios as scenario}<tr>
      <td>{scenario.name === "recrawl" ? scenario.mode === "conditional" ? $t("GET condicional") : $t("GET completo") : scenario.name === "planner" ? $t("Plan en disco") : scenario.name === "traps" ? scenario.mode === "protected" ? $t("Trampas · cuotas activas") : $t("Trampas · referencia") : scenario.name === "diversity" ? $t("Diversidad · igual presupuesto") : $t("Sitemap incremental")}</td>
      <td>{number(scenario.urls || report.configuration.pages)}</td>
      {#each [scenario.metrics.duration_ms, scenario.metrics.cpu_ms, scenario.metrics.rss_after_bytes] as value, index}<td>{number(value.median / (index === 2 ? 1048576 : 1))}<small>{$t("{p0}–{p1} · MAD {p2}", { p0: number(value.min / (index === 2 ? 1048576 : 1)), p1: number(value.max / (index === 2 ? 1048576 : 1)), p2: number(value.mad / (index === 2 ? 1048576 : 1)) })}</small></td>{/each}
    </tr>{/each}
  </tbody></table></div>
  <p>{$t("La comparación de CPU y tiempo pertenece a esta cohorte: {p0} % y {p1} % de reducción. No representa una ganancia global del crawler.", { p0: number(report.comparisons.cpu_ms), p1: number(report.comparisons.duration_ms) })}</p>
  {#if report.comparisons.trap_requests !== undefined}<p>{$t("Trampas: {p0} % menos solicitudes en el corpus controlado, conservando las 200 páginas legítimas.", { p0: number(report.comparisons.trap_requests) })}</p>{/if}
  {#if diversity?.length === 2}<p>{$t("Diversidad a igual presupuesto de {p0} URLs: {p1} → {p2} sitios representados al reservar turnos para exploración.", { p0: number(diversity[0].urls), p1: number(diversity[0].tenants), p2: number(diversity[1].tenants) })}</p>{/if}
  <p>{$t("Recuperación del planner: {p0} cierres forzados, con 100.000 candidatos conservados en cada recuperación. Caché: {p1} MiB.", { p0: report.recovery.length, p1: number(report.configuration.planner_cache_bytes / 1048576) })}</p>
  <p class="status">{$t("Aceptación del banco: {p0} · Código {p1} · {p2}", { p0: report.acceptance.passed ? "verificada" : "pendiente", p1: report.code.commit.slice(0, 12), p2: report.code.source_sha256.slice(0, 12) })}</p>
</section>
<style>
  h3 { font-size: 14px; margin: 24px 0 12px; } p { font-size: 12px; color: #697386; line-height: 1.6; }
  .comparison { display:flex; align-items:center; gap:24px; padding:20px; background:#f6f8fb; border:1px solid #e7eaf0; border-radius:4px; margin:20px 0; }
  .comparison strong { font-size:28px; font-weight:500; white-space:nowrap; } .comparison span { font-size:13px; max-width:650px; }
  .table-scroll { overflow-x:auto; } table { border-collapse:collapse; width:100%; font-size:12px; } th,td { padding:12px; border-bottom:1px solid #e7eaf0; text-align:left; } th { font-weight:600; } small { display:block; margin-top:6px; color:#697386; font-size:11px; } .status { margin-top:20px; }
</style>
