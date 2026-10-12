<script lang="ts">
  import { onMount } from "svelte";
  import TransportBenchmarkPanel from "./TransportBenchmarkPanel.svelte";
  import type {
    BenchmarkReport,
    Distribution,
    BenchmarkScenario,
    BenchmarkRun,
    TransportBenchmarkReport,
  } from "./benchmark-types";

  let { onavailability }: { onavailability: (available: boolean, profile?: string) => void } =
    $props();
  let report = $state<BenchmarkReport | null>(null);
  let transportReport = $state<TransportBenchmarkReport | null>(null);
  let enabled = $state(false);
  let error = $state("");
  let loading = $state(false);
  let selectedIndex = $state(0);
  let controller: AbortController | null = null;
  const selected = $derived(report?.scenarios[selectedIndex]);
  const profile = $derived(transportReport ? "C04" : report?.configuration.acceptance_profile === "c03" ? "C03" : report?.configuration.acceptance_profile === "c02" ? "C02" : "C01");
  const formatter = new Intl.NumberFormat("es-CO", {
    maximumFractionDigits: 2,
  });
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
              `${errorNames[key] ?? key.replaceAll("_", " ")}: ${display(count)}`,
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
    return new Intl.DateTimeFormat("es-CO", {
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
        transportReport = null;
        error = "";
        return;
      }
      if (!response.ok)
        throw new Error(data.error || "No se pudo consultar el informe del banco.");
      if (data.report.kind === "crawler_transport_benchmark") {
        transportReport = data.report;
        report = null;
      } else {
        report = data.report;
        transportReport = null;
      }
      onavailability(true, profile);
      if (selectedIndex >= (report?.scenarios.length ?? 0)) selectedIndex = 0;
      error = "";
    } catch (cause) {
      // A network failure should not add benchmark UI to an unconfigured server.
      if (enabled)
        error =
          cause instanceof Error && cause.name !== "AbortError"
            ? cause.message
            : "La consulta del informe del banco agotó el tiempo de espera.";
    } finally {
      window.clearTimeout(timeout);
      loading = false;
    }
  }
  function download() {
    const currentReport = transportReport ?? report;
    if (!currentReport) return;
    const url = URL.createObjectURL(
      new Blob([JSON.stringify(currentReport, null, 2)], { type: "application/json" }),
    );
    const link = document.createElement("a");
    link.href = url;
    link.download = `crawler-${profile.toLowerCase()}-${Date.parse(currentReport.generated_at)}.json`;
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

{#if enabled}
  <section
    id="banco-c01"
    class="panel benchmark-panel"
    aria-labelledby="benchmark-heading"
  >
    <div class="benchmark-heading">
      <div>
        <span class="benchmark-eyebrow">BANCO LOCAL · {profile}</span>
        <h2 id="benchmark-heading">Capacidad y recursos</h2>
        <p>
          Pruebas controladas en loopback. Sus peticiones y páginas pertenecen
          exclusivamente a este informe.
        </p>
      </div>
      <div class="benchmark-actions">
        <button class="button" onclick={refresh} disabled={loading}
          >Actualizar banco</button
        >
        <button class="button" onclick={download} disabled={!report && !transportReport}
          >Descargar {profile}</button
        >
      </div>
    </div>
    {#if error}<p class="benchmark-error" role="status">
        {error}
        {report || transportReport ? "Se conserva el último informe leído." : ""}
      </p>{/if}
    {#if transportReport}
      <TransportBenchmarkPanel report={transportReport} />
    {:else if report}
      <div class="benchmark-summary">
        <span class="benchmark-result" class:passed={report.acceptance.passed}>
          {report.acceptance.passed
            ? "Aceptación verificada"
            : "Aceptación pendiente"}
        </span>
        <span
          >{report.scenarios.length} escenarios · {date(report.generated_at)} (Bogotá)</span
        >
        <span>Mediana · MAD · rango mínimo–máximo</span>
      </div>
      <p class="benchmark-acceptance-note">
        La aceptación verifica el banco y la conciliación de peticiones; no
        significa ausencia de errores de rastreo. El escenario adversarial
        incluye errores esperados. El detalle por repetición conserva los casos
        que pueden quedar ocultos en la mediana.
      </p>
      <div class="benchmark-scroll">
        <table class="benchmark-table">
          <caption
            >Resultados por escenario y observador. MAD es la desviación
            absoluta mediana.</caption
          >
          <thead
            ><tr
              ><th>Escenario</th><th>Orígenes</th><th>Observador</th><th
                >Repeticiones</th
              >
              {#each resources as resource}<th>{resource.title}</th>{/each}<th
                >Servidor</th
              >
            </tr></thead
          >
          <tbody
            >{#each report.scenarios as scenario}<tr>
                <th scope="row">{scenario.name}</th><td>{scenario.origins}</td
                ><td>{observer(scenario)}</td><td
                  >{scenario.repetitions}</td
                >
                {#each resources as resource}
                  {@const metric = distribution(scenario, resource.key)}
                  <td class="benchmark-numeric"
                    ><strong>{display(metric?.median, resource.unit)}</strong>
                    {#if metric}<small
                        >MAD {display(metric.mad, resource.unit)}</small
                      ><small
                        >{display(metric.min, resource.unit)}–{display(
                          metric.max,
                          resource.unit,
                        )}</small
                      >{/if}
                  </td>
                {/each}
                <td class:benchmark-failed={!scenario.requests_match}
                  >{scenario.requests_match ? "Coincide" : "No coincide"}</td
                >
              </tr>{/each}</tbody
          >
        </table>
      </div>
      <div class="benchmark-details-grid">
        <div class="benchmark-detail">
          <h3>Comparación del observador</h3>
          <p>
            {#if profile === "C03"}
              Asíncrono frente a sincrónico legado. Reducción medida de llamadas
              de escritura, en total y por actualización: un valor positivo
              indica menos escrituras. Se conservan las diferencias de carga y decisiones.
            {:else if profile === "C02"}
              Diferencia de duración mediana con y sin observador. La comparación
              conserva la equivalencia de peticiones, páginas y decisiones publicada.
            {:else}
              Diferencia observada de duración mediana. El informe C01 histórico
              incluye cambios de pausas y presupuestos anteriores a C02; no estima
              sobrecoste puro. Sólo se compara con peticiones y páginas iguales.
            {/if}
          </p>
          <div class="benchmark-scroll">
            <table class="benchmark-table">
              <thead
                ><tr
                  ><th>Escenario</th><th>Orígenes</th><th>Duración</th>
                  {#if profile === "C03"}<th>Reducción de escrituras</th><th>Reducción por actualización</th>{/if}<th
                    >Misma carga</th
                  ></tr
                ></thead
              >
              <tbody
                >{#each report.comparisons ?? [] as comparison}<tr>
                    <th scope="row">{comparison.name}</th><td
                      >{comparison.origins}</td
                    >
                    <td
                      >{!comparison.requests_equal ||
                      !comparison.saved_pages_equal || comparison.behavior_equivalent === false
                        ? "No comparable"
                        : comparison.duration_overhead_pct == null
                          ? "—"
                          : `${comparison.duration_overhead_pct > 0 ? "+" : ""}${display(comparison.duration_overhead_pct)} %`}</td
                    >
                    {#if profile === "C03"}
                      {#each [comparison.write_calls_reduction_pct, comparison.writes_per_update_reduction_pct] as metric}
                        <td class="benchmark-numeric"><strong>{display(metric?.median, "%")}</strong>
                          {#if metric}<small>MAD {display(metric.mad, "%")}</small><small>{display(metric.min, "%")}–{display(metric.max, "%")}</small>{/if}
                        </td>
                      {/each}
                    {/if}
                    <td
                      class:benchmark-failed={!comparison.requests_equal ||
                        !comparison.saved_pages_equal || comparison.behavior_equivalent === false}
                    >
                      {comparison.behavior_equivalent === false
                        ? "Decisiones o trabajo diferentes"
                        : comparison.requests_equal && comparison.saved_pages_equal
                          ? comparison.behavior_equivalent ? "Trabajo y decisiones iguales" : "Peticiones y páginas iguales"
                          : "Peticiones o páginas diferentes"}
                    </td>
                  </tr>{:else}<tr
                    ><td colspan={profile === "C03" ? 6 : 4}>Sin comparación publicada.</td></tr
                  >{/each}</tbody
              >
            </table>
          </div>
        </div>
        <div class="benchmark-detail">
          <div class="benchmark-stage-heading">
            <h3>Etapas y espera</h3>
            <label
              ><span class="sr-only">Escenario para inspeccionar etapas</span
              ><select bind:value={selectedIndex}>
                {#each report.scenarios as scenario, index}<option value={index}
                    >{scenario.name} · {scenario.origins} orígenes · {observer(scenario)}</option
                  >{/each}
              </select></label
            >
          </div>
          <p>
            Tiempo acumulado por etapa entre trabajadores; puede superar la
            duración total y contener etapas anidadas.
          </p>
          <div class="benchmark-scroll">
            <table class="benchmark-table">
              <thead
                ><tr
                  ><th>Etapa</th><th>Mediana</th><th>MAD</th><th>Rango</th></tr
                ></thead
              >
              <tbody
                >{#each Object.entries(selected?.metrics.stages_ms ?? {}) as [key, metric]}<tr
                  >
                    <th scope="row"
                      >{stageNames[key] ?? key.replaceAll("_", " ")}</th
                    ><td>{display(metric.median, "ms")}</td><td
                      >{display(metric.mad, "ms")}</td
                    ><td
                      >{display(metric.min, "ms")}–{display(
                        metric.max,
                        "ms",
                      )}</td
                    >
                  </tr>{:else}<tr
                    ><td colspan="4">Sin etapas publicadas.</td></tr
                  >{/each}</tbody
              >
            </table>
          </div>
          {#if selected}
            {#if selected.metrics.observer_stats}
              <h3 class="benchmark-resource-title">Publicación del observador</h3>
              <div class="benchmark-scroll">
                <table class="benchmark-table" aria-label="Recursos del observador por escenario">
                  <thead><tr><th>Métrica</th><th>Mediana</th><th>MAD</th><th>Rango</th></tr></thead>
                  <tbody>
                    {#each Object.entries(selected.metrics.observer_stats) as [key, metric]}
                      {@const unit = key.endsWith("_ms") ? "ms" : key === "bytes_written" ? "bytes" : ""}
                      <tr><th scope="row">{observerStatsNames[key] ?? key.replaceAll("_", " ")}</th>
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
              Errores por repetición
            </h3>
            <div class="benchmark-scroll">
              <table
                class="benchmark-table"
                aria-labelledby="benchmark-run-errors-heading"
              >
                <thead
                  ><tr
                    ><th>Repetición</th><th>Guardadas</th><th
                      >Peticiones cliente / servidor</th
                    ><th>Fallos de petición</th><th>Errores registrados</th></tr
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
                      ><td colspan="5">Sin detalle por repetición publicado.</td
                      ></tr
                    >{/each}
                </tbody>
              </table>
            </div>
            <h3 class="benchmark-resource-title">
              Recursos del escenario seleccionado
            </h3>
            <p>
              Recursos del proceso crawler; excluyen el servidor de pruebas y el
              dashboard. Los picos de hilos y descriptores dependen del
              intervalo de muestreo.
            </p>
            <div class="benchmark-scroll">
              <table class="benchmark-table">
                <thead
                  ><tr
                    ><th>Recurso</th><th>Mediana</th><th>MAD</th><th>Rango</th
                    ></tr
                  ></thead
                >
                <tbody
                  >{#each additionalResources as resource}
                    {@const metric = distribution(selected, resource.key)}
                    {#if metric}<tr
                        ><th scope="row">{resource.title}</th><td
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
          >Código, corpus, hardware y configuración de esta medición</summary
        >
        <dl>
          <div>
            <dt>Commit</dt>
            <dd>
              {report.code?.commit ?? "No publicado"}{report.code?.dirty
                ? " · cambios locales"
                : ""}
            </dd>
          </div>
          <div>
            <dt>SHA-256 del corpus</dt>
            <dd>{report.corpus?.sha256 ?? "No publicado"}</dd>
          </div>
          <div>
            <dt>Archivos del corpus</dt>
            <dd>{report.corpus?.files?.length ?? "No publicado"}</dd>
          </div>
        </dl>
        <div class="benchmark-details-grid">
          <div>
            <h3>Configuración</h3>
            <pre>{JSON.stringify(report.configuration, null, 2)}</pre>
          </div>
          <div>
            <h3>Hardware</h3>
            <pre>{JSON.stringify(report.hardware ?? {}, null, 2)}</pre>
          </div>
          <div>
            <h3>Huella del código</h3>
            <pre>{JSON.stringify(report.code ?? {}, null, 2)}</pre>
          </div>
          <div>
            <h3>Comprobaciones de aceptación</h3>
            <pre>{JSON.stringify(report.acceptance, null, 2)}</pre>
          </div>
        </div>
      </details>
    {:else}<p class="benchmark-empty">
        Esperando el informe de la ejecución local C01.
      </p>{/if}
  </section>
{/if}
