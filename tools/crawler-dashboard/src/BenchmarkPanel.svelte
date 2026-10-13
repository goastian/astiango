<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import { onMount } from "svelte";
  import { benchmarkNames } from "./navigation";
  import ResilienceBenchmarkPanel from "./ResilienceBenchmarkPanel.svelte";
  import TransportBenchmarkPanel from "./TransportBenchmarkPanel.svelte";
  import BodyBenchmarkPanel from "./BodyBenchmarkPanel.svelte";
  import PipelineBenchmarkPanel from "./PipelineBenchmarkPanel.svelte";
  import SchedulerBenchmarkPanel from "./SchedulerBenchmarkPanel.svelte";
  import CaptureFrontierBenchmarkPanel from "./CaptureFrontierBenchmarkPanel.svelte";
  import SpoolBenchmarkPanel from "./SpoolBenchmarkPanel.svelte";
  import ExtractionBenchmarkPanel from "./ExtractionBenchmarkPanel.svelte";
  import type {
    ResilienceBenchmarkReport,
    BenchmarkReport,
    Distribution,
    BenchmarkScenario,
    BenchmarkRun,
    TransportBenchmarkReport,
    BodyBenchmarkReport,
    ExtractionBenchmarkReport,
    PipelineBenchmarkReport,
    SpoolBenchmarkReport,
    CaptureFrontierBenchmarkReport,
    SchedulerBenchmarkReport,
  } from "./benchmark-types";

  let { visible = true, onavailability = () => {} }: { visible?: boolean; onavailability?: (available: boolean, profile?: string) => void } =
    $props();
  let resilienceReport = $state<ResilienceBenchmarkReport | null>(null);
  let report = $state<BenchmarkReport | null>(null);
  let transportReport = $state<TransportBenchmarkReport | null>(null);
  let bodyReport = $state<BodyBenchmarkReport | null>(null);
  let extractionReport = $state<ExtractionBenchmarkReport | null>(null);
  let pipelineReport = $state<PipelineBenchmarkReport | null>(null);
  let spoolReport = $state<SpoolBenchmarkReport | null>(null);
  let schedulerReport = $state<SchedulerBenchmarkReport | null>(null);
  let captureFrontierReport = $state<CaptureFrontierBenchmarkReport | null>(null);
  let enabled = $state(false);
  let error = $state("");
  let loading = $state(false);
  let ready = $state(false);
  let selectedIndex = $state(0);
  let controller: AbortController | null = null;
  const selected = $derived(report?.scenarios[selectedIndex]);
  const profile = $derived(resilienceReport ? "C14–C20" : schedulerReport ? "C11–C13" : captureFrontierReport ? "C09–C10" : spoolReport ? "C08" : pipelineReport ? "C07" : extractionReport ? "C06" : bodyReport ? "C05" : transportReport ? "C04" : report?.configuration.acceptance_profile === "c03" ? "C03" : report?.configuration.acceptance_profile === "c02" ? "C02" : "C01");
  const name = $derived(benchmarkNames[profile] ?? "Capacidad y recursos");
  const formatter = $derived(new Intl.NumberFormat($formatLocale, {
    maximumFractionDigits: 2,
  }));
  const stageNames: Record<string, string> = {
    dns: "DNS",
    http_headers: "Red · cabeceras HTTP",
    http_body: "Red · cuerpo HTTP",
    politeness: "Espera · cortesía",
    politeness_wait: "Espera · cortesía",
    scheduler_wait: "Espera · planificador",
    parse: "Análisis HTML",
    html_parse: "Análisis HTML",
    warc_write: "Disco · escritura WARC",
    sink: "Compresión y escritura WARC",
    queue_wait: "Espera · cola de trabajos",
    disk: "Disco",
    observer: "Observador",
    robots: "Robots",
    sleep: "Espera",
  };
  const errorNames: Record<string, string> = {
    http_404: "HTTP 404 · no encontrado",
    http_500: "HTTP 500 · error del servidor",
    http_429: "HTTP 429 · límite de peticiones",
    content_too_large: "Cuerpo superior al límite",
    other: "Otros errores",
  };
  const observerStatsNames: Record<string, string> = {
    updates: "Actualizaciones recibidas", snapshot_writes: "Publicaciones atómicas",
    write_calls: "Llamadas de escritura", bytes_written: "Bytes escritos",
    events_total: "Eventos recibidos", events_dropped: "Eventos fuera del buffer",
    errors: "Errores de publicación", pending_updates: "Actualizaciones pendientes",
    max_publish_latency_ms: "Demora máxima de publicación",
    last_publish_latency_ms: "Demora de última publicación",
    legacy_write_estimate: "Escrituras históricas estimadas",
  };
  function runErrors(run: BenchmarkRun) {
    if (!run.errors) return "No publicado";
    const entries = Object.entries(run.errors).filter(([, count]) => count > 0);
    return entries.length
      ? entries
          .map(
            ([key, count]) =>
              `${$t(errorNames[key] ?? key.replaceAll("_", " "))}: ${display(count)}`,
          )
          .join(" · ")
      : "Sin errores registrados";
  }
  const resources = [
    { key: "duration_ms", title: "Duración", unit: "ms" },
    { key: "cpu_seconds", title: "CPU", unit: "s" },
    { key: "rss_peak_bytes", title: "RSS máximo", unit: "bytes" },
    { key: "requests", title: "Peticiones", unit: "" },
    { key: "saved_pages", title: "Guardadas", unit: "" },
  ];
  const additionalResources = [
    { key: "read_bytes", title: "Lecturas de disco", unit: "bytes" },
    { key: "write_bytes", title: "Escrituras de disco", unit: "bytes" },
    { key: "syscr", title: "Llamadas de lectura", unit: "" },
    { key: "syscw", title: "Llamadas de escritura", unit: "" },
    { key: "rchar", title: "Bytes leídos por llamadas", unit: "bytes" },
    { key: "wchar", title: "Bytes escritos por llamadas", unit: "bytes" },
    { key: "fds_peak", title: "Descriptores · pico muestreado", unit: "" },
    { key: "threads_peak", title: "Hilos · pico muestreado", unit: "" },
    { key: "saved_bytes", title: "Contenido guardado", unit: "bytes" },
    { key: "process_wall_ms", title: "Duración del proceso", unit: "ms" },
  ];
  function display(value: number | undefined, unit = "") {
    if (value == null || !Number.isFinite(value)) return "—";
    if (unit === "bytes") {
      if (value < 1024) return `${formatter.format(value)} B`;
      if (value < 1024 * 1024) return `${formatter.format(value / 1024)} KiB`;
      return `${formatter.format(value / 1024 / 1024)} MiB`;
    }
    return `${formatter.format(value)}${unit ? ` ${unit}` : ""}`;
  }
  function distribution(scenario: BenchmarkScenario, key: string) {
    return scenario.metrics[key] as Distribution | undefined;
  }
  function observer(scenario: BenchmarkScenario) {
    if (scenario.observer_mode === "legacy_sync") return "Sincrónico legado";
    if (scenario.observer_mode === "async") return "Asíncrono";
    if (scenario.observer_mode === "off") return "Sin observador";
    const value = scenario.observer;
    return value === true || value === "on" || value === "enabled"
      ? "Con observador"
      : "Sin observador";
  }
  function date(value: string) {
    return new Intl.DateTimeFormat($formatLocale, {
      dateStyle: "medium",
      timeStyle: "short",
      timeZone: "America/Bogota",
    }).format(new Date(value));
  }
  async function refresh() {
    if (loading) return;
    loading = true;
    controller = new AbortController();
    const timeout = window.setTimeout(() => controller?.abort(), 8000);
    try {
      const response = await fetch("/api/benchmarks", {
        cache: "no-store",
        signal: controller.signal,
      });
      const data = await response.json();
      enabled = !(
        response.status === 404 && data.code === "BENCHMARK_DISABLED"
      );
      onavailability(enabled);
      if (!enabled) {
        report = null;
        resilienceReport = null;
        transportReport = null;
        bodyReport = null;
        extractionReport = null;
        pipelineReport = null;
        spoolReport = null;
        captureFrontierReport = null;
      schedulerReport = null;
        error = "";
        return;
      }
      if (!response.ok)
        throw new Error(data.error || "No se pudo consultar el informe del banco.");
      extractionReport = null;
      pipelineReport = null;
      spoolReport = null;
      captureFrontierReport = null;
      schedulerReport = null;
      resilienceReport = null;
      if (data.report.kind === "crawler_resilience_benchmark") {
        resilienceReport = data.report; bodyReport = null; transportReport = null; report = null;
      } else if (data.report.kind === "crawler_scheduler_benchmark") {
        schedulerReport = data.report; bodyReport = null; transportReport = null; report = null;
      } else if (data.report.kind === "crawler_capture_frontier_benchmark") {
        captureFrontierReport = data.report;
        bodyReport = null;
        transportReport = null;
        report = null;
      } else if (data.report.kind === "crawler_spool_benchmark") {
        spoolReport = data.report;
        bodyReport = null;
        transportReport = null;
        report = null;
      } else if (data.report.kind === "crawler_pipeline_benchmark") {
        pipelineReport = data.report;
        bodyReport = null;
        transportReport = null;
        report = null;
      } else if (data.report.kind === "crawler_extraction_benchmark") {
        extractionReport = data.report;
        bodyReport = null;
        transportReport = null;
        report = null;
      } else if (data.report.kind === "crawler_body_benchmark") {
        bodyReport = data.report;
        transportReport = null;
        report = null;
      } else if (data.report.kind === "crawler_transport_benchmark") {
        transportReport = data.report;
        bodyReport = null;
        report = null;
      } else {
        report = data.report;
        transportReport = null;
        bodyReport = null;
      }
      onavailability(true, profile);
      if (selectedIndex >= (report?.scenarios.length ?? 0)) selectedIndex = 0;
      error = "";
    } catch (cause) {
      error =
          cause instanceof Error && cause.name !== "AbortError"
            ? cause.message
            : "La consulta del informe del banco agotó el tiempo de espera.";
    } finally {
      window.clearTimeout(timeout);
      loading = false;
      ready = true;
    }
  }
  function download() {
    const currentReport = resilienceReport ?? schedulerReport ?? captureFrontierReport ?? spoolReport ?? pipelineReport ?? extractionReport ?? bodyReport ?? transportReport ?? report;
    if (!currentReport) return;
    const url = URL.createObjectURL(
      new Blob([JSON.stringify(currentReport, null, 2)], { type: "application/json" }),
    );
    const link = document.createElement("a");
    link.href = url;
    const slug = $t(name).normalize("NFD").replace(/[\u0300-\u036f]/g, "").toLowerCase().replace(/[^a-z0-9]+/g, "-");
    link.download = `crawler-${slug || "measurements"}-${Date.parse(currentReport.generated_at)}.json`;
    link.click();
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  onMount(() => {
    void refresh();
    const interval = window.setInterval(() => {
      if (!document.hidden) void refresh();
    }, 15000);
    return () => {
      window.clearInterval(interval);
      controller?.abort();
    };
  });
</script>

{#if visible}
  <section
    id="capacity-report"
    class="panel benchmark-panel"
    aria-labelledby="benchmark-heading"
  >
    <div class="benchmark-heading">
      <div>
        <span class="benchmark-eyebrow">{$t("Mediciones locales")}</span>
        <h2 id="benchmark-heading">{enabled ? $t(name) : $t("Pruebas de capacidad")}</h2>
        <p>
          {#if resilienceReport}{$t("Revalidación HTTP, cuotas y planificación recuperable con tráfico controlado local.")}
          {:else if schedulerReport}{$t("Turnos por origen y cortesía verificada desde el servidor local. Fixtures separados del estado operativo.")}
          {:else if captureFrontierReport}{$t("Captura HTTP, recuperación y frontera con caché fija. Las pruebas permanecen fuera del estado operativo.")}
          {:else if spoolReport}{$t("Archivo de corpus local y publicación S3 controlada en loopback. Los documentos de prueba pertenecen exclusivamente a este informe.")}
          {:else if extractionReport}{$t("Pruebas de extracción con archivos locales, sin solicitudes de red.")}
          {:else}{$t("Pruebas controladas en loopback. Sus peticiones y páginas pertenecen exclusivamente a este informe.")}{/if}
        </p>
      </div>
      <div class="benchmark-actions">
        <button class="button" onclick={refresh} disabled={loading}
          >{$t("Actualizar mediciones")}</button
        >
        <button class="button" onclick={download} disabled={!resilienceReport && !report && !transportReport && !bodyReport && !extractionReport && !pipelineReport && !spoolReport && !captureFrontierReport && !schedulerReport}
          >{$t("Descargar mediciones")}</button
        >
      </div>
    </div>
    {#if error}<p class="benchmark-error" role="status">
        {$t(error)}
        {resilienceReport || report || transportReport || bodyReport || extractionReport || pipelineReport || spoolReport || captureFrontierReport || schedulerReport ? $t("Se conserva el último informe leído.") : ""}
      </p>{/if}
    {#if resilienceReport}
      <ResilienceBenchmarkPanel report={resilienceReport} />
    {:else if schedulerReport}
      <SchedulerBenchmarkPanel report={schedulerReport} />
    {:else if captureFrontierReport}
      <CaptureFrontierBenchmarkPanel report={captureFrontierReport} />
    {:else if spoolReport}
      <SpoolBenchmarkPanel report={spoolReport} />
    {:else if pipelineReport}
      <PipelineBenchmarkPanel report={pipelineReport} />
    {:else if extractionReport}
      <ExtractionBenchmarkPanel report={extractionReport} />
    {:else if bodyReport}
      <BodyBenchmarkPanel report={bodyReport} />
    {:else if transportReport}
      <TransportBenchmarkPanel report={transportReport} />
    {:else if report}
      <div class="benchmark-summary">
        <span class="benchmark-result" class:passed={report.acceptance.passed}>
          {report.acceptance.passed ? $t("Aceptación verificada") : $t("Aceptación pendiente")}
        </span>
        <span
          >{$t("{p0} escenarios · {p1} (Bogotá)", { p0: report.scenarios.length, p1: date(report.generated_at) })}</span
        >
        <span>{$t("Mediana · MAD · rango mínimo–máximo")}</span>
      </div>
      <p class="benchmark-acceptance-note">
        {$t("La aceptación verifica el banco y la conciliación de peticiones; no significa ausencia de errores de rastreo. El escenario adversarial incluye errores esperados. El detalle por repetición conserva los casos que pueden quedar ocultos en la mediana.")}
      </p>
      <div class="benchmark-scroll">
        <table class="benchmark-table">
          <caption
            >{$t("Resultados por escenario y observador. MAD es la desviación absoluta mediana.")}</caption
          >
          <thead
            ><tr
              ><th>{$t("Escenario")}</th><th>{$t("Orígenes")}</th><th>{$t("Observador")}</th><th
                >{$t("Repeticiones")}</th
              >
              {#each resources as resource}<th>{$t(resource.title)}</th>{/each}<th
                >{$t("Servidor")}</th
              >
            </tr></thead
          >
          <tbody
            >{#each report.scenarios as scenario}<tr>
                <th scope="row">{$t(scenario.name)}</th><td>{scenario.origins}</td
                ><td>{$t(observer(scenario))}</td><td
                  >{scenario.repetitions}</td
                >
                {#each resources as resource}
                  {@const metric = distribution(scenario, resource.key)}
                  <td class="benchmark-numeric"
                    ><strong>{display(metric?.median, resource.unit)}</strong>
                    {#if metric}<small
                        >{$t("MAD {p0}", { p0: display(metric.mad, resource.unit) })}</small
                      ><small
                        >{display(metric.min, resource.unit)}–{display(
                          metric.max,
                          resource.unit,
                        )}</small
                      >{/if}
                  </td>
                {/each}
                <td class:benchmark-failed={!scenario.requests_match}
                  >{scenario.requests_match ? $t("Coincide") : $t("No coincide")}</td
                >
              </tr>{/each}</tbody
          >
        </table>
      </div>
      <div class="benchmark-details-grid">
        <div class="benchmark-detail">
          <h3>{$t("Comparación del observador")}</h3>
          <p>
            {#if profile === "C03"}
              {$t("Asíncrono frente a sincrónico legado. Reducción medida de llamadas de escritura, en total y por actualización: un valor positivo indica menos escrituras. Se conservan las diferencias de carga y decisiones.")}
            {:else if profile === "C02"}
              {$t("Diferencia de duración mediana con y sin observador. La comparación conserva la equivalencia de peticiones, páginas y decisiones publicada.")}
            {:else}
              {$t("Diferencia observada de duración mediana. El informe histórico de capacidad incluye cambios de pausas y presupuestos anteriores a la unificación de resultados; no estima sobrecoste puro. Sólo se compara con peticiones y páginas iguales.")}
            {/if}
          </p>
          <div class="benchmark-scroll">
            <table class="benchmark-table">
              <thead
                ><tr
                  ><th>{$t("Escenario")}</th><th>{$t("Orígenes")}</th><th>{$t("Duración")}</th>
                  {#if profile === "C03"}<th>{$t("Reducción de escrituras")}</th><th>{$t("Reducción por actualización")}</th>{/if}<th
                    >{$t("Misma carga")}</th
                  ></tr
                ></thead
              >
              <tbody
                >{#each report.comparisons ?? [] as comparison}<tr>
                    <th scope="row">{$t(comparison.name)}</th><td
                      >{comparison.origins}</td
                    >
                    <td
                      >{!comparison.requests_equal ||
                      !comparison.saved_pages_equal || comparison.behavior_equivalent === false ? $t("No comparable") : comparison.duration_overhead_pct == null ? "—" : `${comparison.duration_overhead_pct > 0 ? "+" : ""}${display(comparison.duration_overhead_pct)} %`}</td
                    >
                    {#if profile === "C03"}
                      {#each [comparison.write_calls_reduction_pct, comparison.writes_per_update_reduction_pct] as metric}
                        <td class="benchmark-numeric"><strong>{display(metric?.median, "%")}</strong>
                          {#if metric}<small>{$t("MAD {p0}", { p0: display(metric.mad, "%") })}</small><small>{display(metric.min, "%")}–{display(metric.max, "%")}</small>{/if}
                        </td>
                      {/each}
                    {/if}
                    <td
                      class:benchmark-failed={!comparison.requests_equal ||
                        !comparison.saved_pages_equal || comparison.behavior_equivalent === false}
                    >
                      {comparison.behavior_equivalent === false ? $t("Decisiones o trabajo diferentes") : comparison.requests_equal && comparison.saved_pages_equal ? comparison.behavior_equivalent ? $t("Trabajo y decisiones iguales") : $t("Peticiones y páginas iguales") : $t("Peticiones o páginas diferentes")}
                    </td>
                  </tr>{:else}<tr
                    ><td colspan={profile === "C03" ? 6 : 4}>{$t("Sin comparación publicada.")}</td></tr
                  >{/each}</tbody
              >
            </table>
          </div>
        </div>
        <div class="benchmark-detail">
          <div class="benchmark-stage-heading">
            <h3>{$t("Etapas y espera")}</h3>
            <label
              ><span class="sr-only">{$t("Escenario para inspeccionar etapas")}</span
              ><select bind:value={selectedIndex}>
                {#each report.scenarios as scenario, index}<option value={index}
                    >{$t("{p0} · {p1} orígenes · {p2}", { p0: scenario.name, p1: scenario.origins, p2: $t(observer(scenario)) })}</option
                  >{/each}
              </select></label
            >
          </div>
          <p>
            {$t("Tiempo acumulado por etapa entre trabajadores; puede superar la duración total y contener etapas anidadas.")}
          </p>
          <div class="benchmark-scroll">
            <table class="benchmark-table">
              <thead
                ><tr
                  ><th>{$t("Etapa")}</th><th>{$t("Mediana")}</th><th>{$t("MAD")}</th><th>{$t("Rango")}</th></tr
                ></thead
              >
              <tbody
                >{#each Object.entries(selected?.metrics.stages_ms ?? {}) as [key, metric]}<tr
                  >
                    <th scope="row"
                      >{$t(stageNames[key] ?? key.replaceAll("_", " "))}</th
                    ><td>{display(metric.median, "ms")}</td><td
                      >{display(metric.mad, "ms")}</td
                    ><td
                      >{display(metric.min, "ms")}–{display(
                        metric.max,
                        "ms",
                      )}</td
                    >
                  </tr>{:else}<tr
                    ><td colspan="4">{$t("Sin etapas publicadas.")}</td></tr
                  >{/each}</tbody
              >
            </table>
          </div>
          {#if selected}
            {#if selected.metrics.observer_stats}
              <h3 class="benchmark-resource-title">{$t("Publicación del observador")}</h3>
              <div class="benchmark-scroll">
                <table class="benchmark-table" aria-label={$t("Recursos del observador por escenario")}>
                  <thead><tr><th>{$t("Métrica")}</th><th>{$t("Mediana")}</th><th>{$t("MAD")}</th><th>{$t("Rango")}</th></tr></thead>
                  <tbody>
                    {#each Object.entries(selected.metrics.observer_stats) as [key, metric]}
                      {@const unit = key.endsWith("_ms") ? "ms" : key === "bytes_written" ? "bytes" : ""}
                      <tr><th scope="row">{$t(observerStatsNames[key] ?? key.replaceAll("_", " "))}</th>
                        <td>{display(metric.median, unit)}</td><td>{display(metric.mad, unit)}</td>
                        <td>{display(metric.min, unit)}–{display(metric.max, unit)}</td>
                      </tr>
                    {/each}
                  </tbody>
                </table>
              </div>
            {/if}
            <h3
              class="benchmark-resource-title"
              id="benchmark-run-errors-heading"
            >
              {$t("Errores por repetición")}
            </h3>
            <div class="benchmark-scroll">
              <table
                class="benchmark-table"
                aria-labelledby="benchmark-run-errors-heading"
              >
                <thead
                  ><tr
                    ><th>{$t("Repetición")}</th><th>{$t("Guardadas")}</th><th
                      >{$t("Peticiones cliente / servidor")}</th
                    ><th>{$t("Fallos de petición")}</th><th>{$t("Errores registrados")}</th></tr
                  ></thead
                >
                <tbody>
                  {#each selected.runs ?? [] as run, index}
                    <tr>
                      <th scope="row">{run.repetition ?? index + 1}</th>
                      <td>{display(run.saved_pages)}</td>
                      <td class:benchmark-failed={run.requests_match === false}
                        >{display(run.client_requests)} / {display(
                          run.server?.requests,
                        )}</td
                      >
                      <td
                        class:benchmark-failed={(run.request_failures ?? 0) > 0}
                        >{display(run.request_failures)}</td
                      >
                      <td
                        class:benchmark-failed={Object.values(
                          run.errors ?? {},
                        ).some((count) => count > 0)}>{runErrors(run)}</td
                      >
                    </tr>
                  {:else}<tr
                      ><td colspan="5">{$t("Sin detalle por repetición publicado.")}</td
                      ></tr
                    >{/each}
                </tbody>
              </table>
            </div>
            <h3 class="benchmark-resource-title">
              {$t("Recursos del escenario seleccionado")}
            </h3>
            <p>
              {$t("Recursos del proceso crawler; excluyen el servidor de pruebas y el dashboard. Los picos de hilos y descriptores dependen del intervalo de muestreo.")}
            </p>
            <div class="benchmark-scroll">
              <table class="benchmark-table">
                <thead
                  ><tr
                    ><th>{$t("Recurso")}</th><th>{$t("Mediana")}</th><th>{$t("MAD")}</th><th>{$t("Rango")}</th
                    ></tr
                  ></thead
                >
                <tbody
                  >{#each additionalResources as resource}
                    {@const metric = distribution(selected, resource.key)}
                    {#if metric}<tr
                        ><th scope="row">{$t(resource.title)}</th><td
                          >{display(metric.median, resource.unit)}</td
                        ><td>{display(metric.mad, resource.unit)}</td><td
                          >{display(metric.min, resource.unit)}–{display(
                            metric.max,
                            resource.unit,
                          )}</td
                        ></tr
                      >{/if}
                  {/each}</tbody
                >
              </table>
            </div>
          {/if}
        </div>
      </div>
      <details class="benchmark-provenance">
        <summary
          >{$t("Código, corpus, hardware y configuración de esta medición")}</summary
        >
        <dl>
          <div>
            <dt>{$t("Commit")}</dt>
            <dd>
              {report.code?.commit ?? $t("No publicado")}{report.code?.dirty ? $t("· cambios locales") : ""}
            </dd>
          </div>
          <div>
            <dt>{$t("SHA-256 del corpus")}</dt>
            <dd>{report.corpus?.sha256 ?? $t("No publicado")}</dd>
          </div>
          <div>
            <dt>{$t("Archivos del corpus")}</dt>
            <dd>{report.corpus?.files?.length ?? $t("No publicado")}</dd>
          </div>
        </dl>
        <div class="benchmark-details-grid">
          <div>
            <h3>{$t("Configuración")}</h3>
            <pre>{JSON.stringify(report.configuration, null, 2)}</pre>
          </div>
          <div>
            <h3>{$t("Hardware")}</h3>
            <pre>{JSON.stringify(report.hardware ?? {}, null, 2)}</pre>
          </div>
          <div>
            <h3>{$t("Huella del código")}</h3>
            <pre>{JSON.stringify(report.code ?? {}, null, 2)}</pre>
          </div>
          <div>
            <h3>{$t("Comprobaciones de aceptación")}</h3>
            <pre>{JSON.stringify(report.acceptance, null, 2)}</pre>
          </div>
        </div>
      </details>
    {:else}<p class="benchmark-empty">
      {#if !ready}{$t("Consultando las mediciones locales…")}{:else if !enabled && !error}{$t("No hay un informe local seleccionado. Selecciona un informe al iniciar el dashboard para comparar capacidad y recursos.")}{:else}{$t("Esperando el informe de la ejecución local.")}{/if}
      </p>{/if}
  </section>
{/if}
