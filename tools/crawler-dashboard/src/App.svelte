<script lang="ts">
  import { onMount } from "svelte";
  import Icon from "./Icon.svelte";
  import BenchmarkPanel from "./BenchmarkPanel.svelte";
  import RequestOutcomes from "./RequestOutcomes.svelte";
  import {
    buildHistoryBars,
    summarizeOutcomes,
    buildSiteOutcomes,
  } from "./metrics";
  import type { StatusResponse, Site, SiteStatus } from "./types";

  let response = $state<StatusResponse | null>(null);
  let error = $state("");
  let loading = $state(false);
  let paused = $state(false);
  let now = $state(Date.now());
  let query = $state("");
  let filter = $state("all");
  let eventFilter = $state("all");
  let selectedHost = $state("");
  let windowMinutes = $state(15);
  let showConfig = $state(false);
  let benchmarkAvailable = $state(false);
  let lastRead = $state<string | null>(null);
  let abortController: AbortController | null = null;

  const number = new Intl.NumberFormat("es-CO");
  const clock = new Intl.DateTimeFormat("es-CO", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
    timeZone: "America/Bogota",
  });
  const calendar = new Intl.DateTimeFormat("es-CO", {
    day: "2-digit",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZone: "America/Bogota",
  });
  const labels: Record<SiteStatus, string> = {
    queued: "En cola",
    crawling: "Rastreando",
    waiting: "En espera",
    completed: "Finalizado",
    stopped: "Detenido",
    deferred: "Aplazado",
    cancelled: "Cancelado",
    storage_failed: "Fallo de almacenamiento",
    failed: "Fallido",
  };
  const phases = {
    running: "En ejecución",
    idle: "Esperando sitios",
    completed: "Completado",
    stopped: "Detenido",
    failed: "Error de ejecución",
  };
  const snapshot = $derived(response?.snapshot);
  const activeProcess = $derived(
    snapshot?.phase === "running" || snapshot?.phase === "idle",
  );
  const stale = $derived(
    !!snapshot && now - Date.parse(snapshot.updated_at) > 15_000,
  );
  const offline = $derived(
    response?.meta.process_alive === false && activeProcess,
  );
  const live = $derived(
    !!snapshot && activeProcess && !stale && !offline && !error && !paused,
  );
  const activeSites = $derived(
    snapshot?.sites.filter(
      (site) => site.status === "crawling" || site.status === "waiting",
    ).length ?? 0,
  );
  const filteredSites = $derived(
    (snapshot?.sites ?? [])
      .filter((site) => {
        const matchesQuery = `${site.host} ${site.current_url ?? ""}`
          .toLowerCase()
          .includes(query.toLowerCase().trim());
        const matchesFilter =
          filter === "all" ||
          (filter === "active" &&
            ["crawling", "waiting"].includes(site.status)) ||
          (filter === "errors" && incidents(site) > 0) ||
          (filter === "completed" && site.status === "completed");
        return matchesQuery && matchesFilter;
      })
      .sort((a, b) => {
        const priority = {
          crawling: 0,
          waiting: 1,
          queued: 2,
          failed: 3,
          storage_failed: 3,
          deferred: 4,
          cancelled: 5,
          stopped: 6,
          completed: 7,
        };
        return (
          priority[a.status] - priority[b.status] ||
          a.host.localeCompare(b.host)
        );
      }),
  );
  const selected = $derived(
    snapshot?.sites.find((site) => site.host === selectedHost) ??
      snapshot?.sites.find((site) => site.status === "crawling") ??
      snapshot?.sites[0],
  );
  const events = $derived(
    (snapshot?.recent_events ?? []).filter(
      (event) =>
        eventFilter === "all" ||
        (eventFilter === "errors" && event.level !== "info") ||
        (eventFilter === "site" && event.host === selected?.host),
    ),
  );
  const statusText = $derived(
    error
      ? "Sin lectura del crawler"
      : !snapshot
        ? "Sin estado"
        : offline
          ? "Proceso sin conexión"
          : paused
            ? "Lectura pausada"
            : stale && activeProcess
              ? "Estado sin actualizar"
              : phases[snapshot.phase],
  );
  const elapsed = $derived(
    snapshot
      ? Math.max(
          0,
          (snapshot.finished_at ? Date.parse(snapshot.finished_at) : now) -
            Date.parse(snapshot.started_at),
        )
      : 0,
  );
  const recentHistory = $derived(
    (snapshot?.history ?? []).filter(
      (point) =>
        snapshot &&
        Date.parse(snapshot.updated_at) - Date.parse(point.at) <= 60_000,
    ),
  );
  const rate = $derived.by(() => {
    if (recentHistory.length < 2) return null;
    const first = recentHistory[0],
      last = recentHistory[recentHistory.length - 1];
    const seconds = (Date.parse(last.at) - Date.parse(first.at)) / 1000;
    return seconds > 0
      ? Math.max(0, ((last.downloaded - first.downloaded) * 60) / seconds)
      : null;
  });

  const historyBars = $derived(
    buildHistoryBars(
      snapshot?.history ?? [],
      snapshot?.updated_at ?? new Date(now).toISOString(),
      windowMinutes,
    ),
  );
  const outcomes = $derived(
    snapshot ? summarizeOutcomes(snapshot.counters) : null,
  );
  const siteOutcomes = $derived(buildSiteOutcomes(snapshot?.sites ?? [], 8));
  const maxSiteTotal = $derived(
    Math.max(1, ...siteOutcomes.map((site) => site.total)),
  );
  const maxDownloads = $derived(
    Math.max(1, ...historyBars.map((point) => point.downloaded)),
  );
  const maxBytes = $derived(
    Math.max(1, ...historyBars.map((point) => point.bytes)),
  );
  const issues = $derived(
    (snapshot?.sites ?? [])
      .filter((site) => incidents(site) > 0)
      .sort((a, b) => incidents(b) - incidents(a)),
  );
  const domainTotal = $derived(
    snapshot?.frontier?.discovered_domains ?? snapshot?.counters.sites ?? 0,
  );
  const domainRows = $derived([
    {
      label: "Finalizados en esta ejecución",
      value: snapshot?.counters.completed_sites ?? 0,
      color: "green",
    },
    { label: "En trabajo ahora", value: activeSites, color: "blue" },
    {
      label: "Pendientes en la cola",
      value: snapshot?.frontier?.pending_domains ?? 0,
      color: "amber",
    },
  ]);
  const domainScale = $derived(
    Math.max(
      4,
      Math.ceil(Math.max(...domainRows.map((row) => row.value)) / 4) * 4,
    ),
  );
  const downloadDelta = $derived(
    historyBars.reduce((sum, bar) => sum + bar.downloaded, 0),
  );
  const byteDelta = $derived(
    historyBars.reduce((sum, bar) => sum + bar.bytes, 0),
  );
  const hasHistory = $derived(historyBars.some((bar) => bar.observed));
  function shortTime(value: number) {
    return clock.format(new Date(value)).slice(0, 5);
  }
  function incidents(site: Site) {
    return site.failed + (site.storage_failed ?? 0) + (site.deferred ?? 0) + (site.cancelled ?? 0);
  }
  function optionalCount(value: number | undefined) {
    return value == null ? "—" : number.format(value);
  }
  function percent(value: number | null | undefined) {
    return value == null
      ? "—"
      : value.toLocaleString("es-CO", { maximumFractionDigits: 1 });
  }
  function exportReport() {
    if (!response) return;
    const blob = new Blob([JSON.stringify(response, null, 2)], {
      type: "application/json",
    });
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `crawler-${response.snapshot.run_id}-${Date.parse(response.snapshot.updated_at)}.json`;
    link.click();
    window.setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  function bytes(value: number | undefined) {
    if (value == null) return "—";
    if (value < 1024) return `${number.format(value)} B`;
    const units = ["KiB", "MiB", "GiB", "TiB"];
    let unit = -1,
      amount = value;
    do {
      amount /= 1024;
      unit++;
    } while (amount >= 1024 && unit < units.length - 1);
    return `${amount.toLocaleString("es-CO", { maximumFractionDigits: 1 })} ${units[unit]}`;
  }
  function time(value: string | null | undefined) {
    return value && Number.isFinite(Date.parse(value))
      ? clock.format(new Date(value))
      : "—";
  }
  function date(value: string | undefined) {
    return value && Number.isFinite(Date.parse(value))
      ? calendar.format(new Date(value))
      : "—";
  }
  function duration(value: number) {
    const seconds = Math.floor(value / 1000);
    if (seconds < 60) return `${seconds} s`;
    if (seconds < 3600)
      return `${Math.floor(seconds / 60)} min ${seconds % 60} s`;
    return `${Math.floor(seconds / 3600)} h ${Math.floor((seconds % 3600) / 60)} min`;
  }
  function progress(site: Site) {
    return site.budget
      ? Math.min(100, (site.attempted / site.budget) * 100)
      : 0;
  }
  function safeLink(url: string) {
    try {
      const parsed = new URL(url);
      return ["http:", "https:"].includes(parsed.protocol) ? url : undefined;
    } catch {
      return undefined;
    }
  }
  async function refresh() {
    if (loading) return;
    loading = true;
    abortController = new AbortController();
    const timeout = window.setTimeout(() => abortController?.abort(), 8000);
    try {
      const result = await fetch("/api/status", {
        cache: "no-store",
        signal: abortController.signal,
      });
      const data = await result.json();
      if (!result.ok)
        throw new Error(
          data.error || "No se pudo consultar el estado del crawler.",
        );
      response = data;
      lastRead = new Date().toISOString();
      error = "";
    } catch (cause) {
      error =
        cause instanceof Error && cause.name !== "AbortError"
          ? cause.message
          : "La lectura del estado agotó el tiempo de espera.";
    } finally {
      window.clearTimeout(timeout);
      loading = false;
    }
  }
  function togglePolling() {
    paused = !paused;
    if (!paused) void refresh();
  }
  onMount(() => {
    void refresh();
    const polling = window.setInterval(() => {
      if (!paused && !document.hidden) void refresh();
    }, 2000);
    const ticking = window.setInterval(() => {
      now = Date.now();
    }, 1000);
    const onVisible = () => {
      if (!document.hidden && !paused) void refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      window.clearInterval(polling);
      window.clearInterval(ticking);
      abortController?.abort();
      document.removeEventListener("visibilitychange", onVisible);
    };
  });
</script>

<svelte:head><title>Crawler · Astian</title></svelte:head>

<a class="skip-link" href="#resumen">Ir al resumen</a>
<div class="workspace">
  <header class="page-header">
    <div class="header-main">
      <div class="page-title">
        <span class="app-symbol"><Icon name="globe" size={25} /></span>
        <h1>Resumen <span>/ Crawler</span></h1>
      </div>
      <div class="header-actions">
        <button
          class="button primary"
          onclick={refresh}
          disabled={loading}
          aria-label="Actualizar estado ahora"
          ><Icon name="refresh" />Actualizar</button
        >
        <button class="button" onclick={togglePolling} aria-pressed={paused}
          ><Icon name={paused ? "play" : "pause"} />{paused
            ? "Reanudar lectura"
            : "Pausar lectura"}</button
        >
        <button class="button" onclick={exportReport} disabled={!snapshot}
          ><Icon name="download" />Descargar informe</button
        >
        <button
          class="button"
          onclick={() => (showConfig = !showConfig)}
          aria-expanded={showConfig}
          aria-controls="config-panel"
          ><Icon name="settings" /><span>Configuración</span></button
        >
      </div>
    </div>
    <div class="report-controls">
      <div class="report-field">
        <span class="control-label">Ejecución actual</span>
        <div class="run-date">
          <Icon name="calendar" /><span>{date(snapshot?.started_at)}</span><span
            class="run-tag">Local</span
          >
        </div>
      </div>
      <label class="report-field"
        ><span class="control-label">Intervalo de los gráficos</span><span
          class="select-wrap"
          ><Icon name="clock" /><select bind:value={windowMinutes}
            ><option value={5}>Últimos 5 minutos</option><option value={15}
              >Últimos 15 minutos</option
            ><option value={60}>Última hora</option></select
          ></span
        ></label
      >
      <div class="run-health">
        <span
          class="connection"
          class:live
          class:warning={!!error || offline || (stale && activeProcess)}
          role="status"><span class="status-dot"></span>{statusText}</span
        ><span>Última lectura {time(lastRead)} · cada 2 s</span>
      </div>
    </div>
    {#if showConfig}
      <section
        class="config-panel"
        id="config-panel"
        aria-label="Configuración activa"
      >
        <div>
          <strong>Configuración activa</strong><span
            >Proceso {snapshot?.pid ?? "—"} · {snapshot
              ? duration(elapsed)
              : "—"} en ejecución</span
          >
        </div>
        <dl>
          <div>
            <dt>Trabajadores</dt>
            <dd>{snapshot?.workers ?? "—"}</dd>
          </div>
          <div>
            <dt>Pausa mínima por sitio</dt>
            <dd>
              {snapshot ? `${snapshot.min_crawl_delay_ms / 1000} s` : "—"}
            </dd>
          </div>
          <div>
            <dt>Intentos por sitio</dt>
            <dd>{snapshot?.pages_per_domain ?? "—"}</dd>
          </div>
          <div>
            <dt>Descubrimiento</dt>
            <dd>{snapshot?.continuous ? "Continuo" : "Por ejecución"}</dd>
          </div>
        </dl>
        <p>
          Se inicia en dominios aleatorios y continúa a través de sus enlaces.
          Respeta las reglas de robots.txt.
        </p>
      </section>
    {/if}
    <nav class="section-nav" aria-label="Secciones del dashboard">
      <a href="#resumen" class="current">Vista general</a><a href="#sitios"
        >Sitios</a
      ><a href="#actividad">Actividad</a><a href="#almacenamiento"
        >Archivos WARC</a
      >{#if benchmarkAvailable}<a href="#banco-c01">Banco C01 · local</a
        >{/if}<span>ASTIAN CRAWLER</span>
    </nav>
  </header>
  <main>
    {#if error || offline || (stale && activeProcess) || paused}
      <div class="notice" role="status">
        <strong
          >{paused && !error && !offline
            ? "Actualización pausada."
            : "Lectura no actualizada."}</strong
        >
        {error ||
          (offline
            ? "El proceso del crawler ya no está activo."
            : paused
              ? "El crawler continúa de forma independiente."
              : "El crawler no ha publicado un estado reciente.")}
        {#if snapshot}Se conserva el último estado recibido.{/if}
      </div>
    {/if}
    <section
      class="overview-grid"
      id="resumen"
      aria-label="Resumen del rastreo"
    >
      <article class="panel metric-panel pages-panel">
        <h2>
          Páginas guardadas <span
            class="info"
            title="Descargas guardadas en archivos WARC durante esta ejecución"
            >i</span
          >
        </h2>
        <div class="metric-value">
          <strong
            >{snapshot
              ? number.format(snapshot.counters.downloaded)
              : "—"}</strong
          ><span class="metric-delta"
            >{hasHistory ? `+ ${number.format(downloadDelta)}` : "—"}</span
          >
        </div>
        <div
          class="mini-chart"
          role="img"
          aria-label={`Páginas guardadas por intervalo en los últimos ${windowMinutes} minutos`}
        >
          <span class="chart-maximum">{number.format(maxDownloads)}</span>
          <div class="mini-bars">
            {#each historyBars as bar}<div
                class="mini-slot"
                class:unobserved={!bar.observed}
                title={bar.observed
                  ? `${shortTime(bar.start)}–${shortTime(bar.end)}: ${bar.downloaded} páginas`
                  : `${shortTime(bar.start)}–${shortTime(bar.end)}: sin medición`}
              >
                <span
                  class="mini-bar blue"
                  style:height={`${(bar.downloaded / maxDownloads) * 100}%`}
                ></span>
              </div>{/each}
          </div>
          <span class="chart-zero">0</span>
        </div>
        <div class="chart-caption">
          <span
            >{historyBars.length ? shortTime(historyBars[0].start) : "—"}</span
          ><span
            >{rate === null
              ? "Esperando medición"
              : `${percent(rate)} pág. / min`}</span
          >
        </div>
      </article>
      <article class="panel metric-panel content-panel">
        <h2>
          Contenido guardado <span
            class="info"
            title="Tamaño del contenido de las páginas antes de comprimir"
            >i</span
          >
        </h2>
        <div class="metric-value">
          <strong class="bytes-value">{bytes(snapshot?.counters.bytes)}</strong
          ><span class="metric-delta purple-text"
            >{hasHistory ? `+ ${bytes(byteDelta)}` : "—"}</span
          >
        </div>
        <div
          class="mini-chart"
          role="img"
          aria-label={`Contenido guardado por intervalo en los últimos ${windowMinutes} minutos`}
        >
          <span class="chart-maximum">{bytes(maxBytes)}</span>
          <div class="mini-bars">
            {#each historyBars as bar}<div
                class="mini-slot"
                class:unobserved={!bar.observed}
                title={bar.observed
                  ? `${shortTime(bar.start)}–${shortTime(bar.end)}: ${bytes(bar.bytes)}`
                  : `${shortTime(bar.start)}–${shortTime(bar.end)}: sin medición`}
              >
                <span
                  class="mini-bar purple"
                  style:height={`${(bar.bytes / maxBytes) * 100}%`}
                ></span>
              </div>{/each}
          </div>
          <span class="chart-zero">0</span>
        </div>
        <div class="chart-caption">
          <span>Antes de comprimir</span><span>Últimos {windowMinutes} min</span
          >
        </div>
      </article>
      <article class="panel success-panel">
        <h2>
          Descargas correctas <span
            class="info"
            title="Páginas guardadas / (guardadas + fallos permanentes + fallos de almacenamiento). Excluye omisiones, aplazamientos y cancelaciones."
            >i</span
          >
        </h2>
        <div
          class="gauge"
          role="img"
          aria-label={outcomes?.successRate == null
            ? "Sin descargas con resultado"
            : `${percent(outcomes.successRate)} por ciento de descargas correctas`}
        >
          <svg viewBox="0 0 240 207" aria-hidden="true"
            ><path
              class="gauge-track"
              d="M49.3 175.7 A100 100 0 1 1 190.7 175.7"
              pathLength="100"
            /><path
              class="gauge-value"
              d="M49.3 175.7 A100 100 0 1 1 190.7 175.7"
              pathLength="100"
              stroke-dasharray={`${outcomes?.successRate ?? 0} 100`}
            /></svg
          >
          <div class="gauge-label">
            <strong
              >{percent(outcomes?.successRate)}<small
                >{outcomes?.successRate != null ? "%" : ""}</small
              ></strong
            ><span>DE LAS DESCARGAS</span>
          </div>
        </div>
        <p class="success-description">
          {snapshot ? number.format(snapshot.counters.downloaded) : "—"} guardadas
          de {outcomes ? number.format(outcomes.settled) : "—"}<br />descargas
          con resultado
        </p>
        <div class="outcome-totals">
          <div>
            <span class="legend-dot green"></span><strong
              >{snapshot?.counters.downloaded ?? "—"}</strong
            ><span>Guardadas</span>
          </div>
          <div>
            <span class="legend-dot red"></span><strong
              >{outcomes ? outcomes.failed + outcomes.storageFailed : "—"}</strong
            ><span>Fallidas</span>
          </div>
          <div>
            <span class="legend-dot amber"></span><strong
              >{snapshot?.counters.skipped ?? "—"}</strong
            ><span>Omitidas</span>
          </div>
        </div>
        <p class="card-footnote">Acumulado de esta ejecución</p>
      </article>
      <article class="panel domains-panel">
        <div class="card-heading">
          <h2>
            Dominios descubiertos <span
              class="info"
              title="Dominios conocidos en la ronda de descubrimiento. La cola se conserva entre reinicios."
              >i</span
            >
          </h2>
          <span class="subtle-badge"
            >{snapshot?.continuous ? "Continuo" : "Por ejecución"}</span
          >
        </div>
        <div class="domain-summary">
          <strong>{snapshot ? number.format(domainTotal) : "—"}</strong>
          <p>dominios conocidos<span>Ronda actual de descubrimiento</span></p>
        </div>
        <div class="domain-chart">
          {#each domainRows as row}<div class="domain-row">
              <div class="domain-row-label">
                <span>{row.label}</span><strong
                  >{snapshot ? number.format(row.value) : "—"}</strong
                >
              </div>
              <div
                class="domain-track"
                role="meter"
                aria-label={`Dominios ${row.label.toLowerCase()}`}
                aria-valuenow={row.value}
                aria-valuemin={0}
                aria-valuemax={domainScale}
              >
                <span
                  class={row.color}
                  style:width={`${Math.min(100, (row.value / domainScale) * 100)}%`}
                ></span>
              </div>
            </div>{/each}
          <div
            class="percentage-axis"
            aria-label="Escala en número de dominios"
          >
            {#each [0, 1, 2, 3, 4] as step}<span
                >{number.format((domainScale * step) / 4)}</span
              >{/each}
          </div>
        </div>
        <div class="legend">
          <span><i class="legend-dot green"></i>Finalizados</span><span
            ><i class="legend-dot blue"></i>En trabajo</span
          ><span><i class="legend-dot amber"></i>En cola</span>
        </div>
      </article>
    </section>
    <RequestOutcomes counters={snapshot?.counters} />
    <div class="analysis-grid">
      <section class="panel issues-panel" aria-labelledby="issues-heading">
        <div class="card-heading">
          <h2 id="issues-heading">Sitios con incidencias</h2>
          <a href="#actividad" onclick={() => (eventFilter = "errors")}
            >Ver actividad <span aria-hidden="true">↗</span></a
          >
        </div>
        <p class="card-description">
          Fallos, aplazamientos y cancelaciones del último trabajo de cada sitio
        </p>
        <div class="issue-list">
          {#each issues.slice(0, 5) as site}<button
              class="issue-row"
              onclick={() => {
                selectedHost = site.host;
                document
                  .getElementById("sitios")
                  ?.scrollIntoView({ behavior: "smooth" });
              }}
              ><span class="issue-icon" aria-hidden="true">!</span><span
                class="issue-description"
                ><strong>{site.host}</strong><span title={site.last_error ?? ""}
                  >{site.last_error ??
                    "Detalle no disponible en el estado actual"}</span
                ></span
              ><span class="issue-line"></span><strong class="issue-count"
                >{number.format(incidents(site))}</strong
              ></button
            >{:else}<div class="empty-chart">
              <span class="empty-check" aria-hidden="true">✓</span><strong
                >{snapshot
                  ? "Sin incidencias en los sitios recientes"
                  : "Esperando datos del crawler"}</strong
              ><span>Las incidencias registradas aparecerán aquí.</span>
            </div>{/each}
        </div>
        <p class="card-footnote">
          {snapshot
            ? `${number.format(snapshot.counters.failed)} fallidas en toda la ejecución`
            : "Sin ejecución disponible"} · hasta 500 sitios recientes
        </p>
      </section>
      <section
        class="panel distribution-panel"
        aria-labelledby="distribution-heading"
      >
        <div class="card-heading">
          <h2 id="distribution-heading">Resultados por sitio</h2>
          <div class="legend compact">
            <span><i class="legend-dot green"></i>Guardadas</span><span
              ><i class="legend-dot amber"></i>Omitidas</span
            ><span><i class="legend-dot red"></i>Fallidas</span>
            <span><i class="legend-dot blue"></i>Aplazadas</span>
            <span><i class="legend-dot neutral"></i>Canceladas</span>
            <span><i class="legend-dot purple"></i>Almacenamiento</span>
          </div>
        </div>
        <div
          class="distribution-chart"
          role="img"
          aria-label="Comparación de páginas guardadas, omitidas, fallidas, aplazadas, canceladas y fallos de almacenamiento por sitio"
        >
          <div class="distribution-axis">
            <span>{number.format(maxSiteTotal)}</span><span
              >{number.format(Math.round(maxSiteTotal / 2))}</span
            ><span>0</span>
          </div>
          <div class="distribution-columns">
            {#each siteOutcomes as site}<div class="distribution-column">
                <div class="column-plot">
                  <div
                    class="stacked-column"
                    style:height={`${(site.total / maxSiteTotal) * 100}%`}
                    title={`${site.host}: ${site.downloaded} guardadas, ${site.skipped} omitidas, ${site.failed} fallidas, ${site.deferred} aplazadas, ${site.cancelled} canceladas, ${site.storageFailed} fallos de almacenamiento`}
                  >
                    <span class="green" style:flex={site.downloaded}
                    ></span><span class="amber" style:flex={site.skipped}
                    ></span><span class="red" style:flex={site.failed}></span>
                    <span class="blue" style:flex={site.deferred}></span>
                    <span class="neutral" style:flex={site.cancelled}></span>
                    <span class="purple" style:flex={site.storageFailed}></span>
                  </div>
                </div>
                <button
                  class="column-label"
                  title={`Inspeccionar ${site.host}`}
                  onclick={() => {
                    selectedHost = site.host;
                    document
                      .getElementById("sitios")
                      ?.scrollIntoView({ behavior: "smooth" });
                  }}>{site.host}</button
                >
              </div>{:else}<div class="empty-chart">
                Los resultados aparecerán al completar las primeras solicitudes.
              </div>{/each}
          </div>
        </div>
        <p class="card-footnote">
          {siteOutcomes.length} sitios con más resultados en el estado actual
        </p>
      </section>
    </div>
    <section
      id="sitios"
      class="sites-section panel"
      aria-labelledby="sites-heading"
    >
      <div class="section-heading">
        <div class="section-title">
          <h2 id="sites-heading">Sitios en la ejecución</h2>
          <span class="count">{snapshot?.sites.length ?? "—"}</span>
        </div>
        <span class="section-caption"
          >{!live && snapshot
            ? "Último estado registrado"
            : snapshot?.continuous
              ? "Descubrimiento continuo"
              : "Trabajo por dominio"}</span
        >
      </div>
      <div class="work-area">
        <div class="sites-main">
          <div class="table-toolbar">
            <div
              class="filters"
              role="group"
              aria-label="Filtrar sitios por estado"
            >
              <button
                class:active={filter === "all"}
                onclick={() => (filter = "all")}
                aria-pressed={filter === "all"}>Todos</button
              >
              <button
                class:active={filter === "active"}
                onclick={() => (filter = "active")}
                aria-pressed={filter === "active"}>En trabajo</button
              >
              <button
                class:active={filter === "errors"}
                onclick={() => (filter = "errors")}
                aria-pressed={filter === "errors"}>Con incidencias</button
              >
              <button
                class:active={filter === "completed"}
                onclick={() => (filter = "completed")}
                aria-pressed={filter === "completed"}>Finalizados</button
              >
            </div>
            <label class="search"
              ><svg
                viewBox="0 0 20 20"
                fill="none"
                stroke="currentColor"
                stroke-width="1.5"
                width="16"
                height="16"
                aria-hidden="true"
                ><circle cx="8.5" cy="8.5" r="5.5" /><path
                  d="m13 13 4 4"
                /></svg
              ><input
                bind:value={query}
                type="search"
                placeholder="Buscar sitio o URL"
                aria-label="Buscar sitio o URL"
              /></label
            >
          </div>
          <div class="table-scroll">
            <table
              class="sites-table"
              class:has-sites={filteredSites.length > 0}
            >
              <thead
                ><tr
                  ><th>Sitio / URL actual</th><th>Estado</th><th class="numeric"
                    >Guardadas</th
                  ><th class="numeric" title="Descargas de páginas fallidas"
                    >Fallidas</th
                  ><th
                    class="budget-col"
                    title="Intentos de descarga de páginas / límite por sitio"
                    >Intentos / límite</th
                  ></tr
                ></thead
              ><tbody>
                {#each filteredSites as site (site.host)}
                  <tr class:row-selected={selected?.host === site.host}
                    ><td
                      ><button
                        class="site-select"
                        onclick={() => (selectedHost = site.host)}
                        aria-pressed={selected?.host === site.host}
                        ><span class="site-host"
                          >{site.host}<span class="row-arrow" aria-hidden="true"
                            >↗</span
                          ></span
                        ><span class="site-url" title={site.current_url ?? ""}
                          >{site.current_url ??
                            (site.status === "queued"
                              ? "Pendiente de asignación"
                              : site.status === "completed"
                                ? "Trabajo del sitio finalizado"
                                : ["deferred", "cancelled", "storage_failed", "failed", "stopped"].includes(site.status)
                                  ? "Sin solicitud en curso"
                                : "Entre solicitudes")}</span
                        ></button
                      ></td
                    ><td
                      ><span
                        class="site-status"
                        class:working={live && site.status === "crawling"}
                        class:waiting={site.status === "waiting"}
                        ><span class="status-dot"></span>{labels[
                          site.status
                        ]}</span
                      ></td
                    ><td class="numeric"
                      >{number.format(site.pages_downloaded)}</td
                    ><td class="numeric" class:error-count={site.failed > 0}
                      >{number.format(site.failed)}</td
                    ><td class="budget-col"
                      ><div class="budget-values">
                        <span>{number.format(site.attempted)}</span><span
                          >/ {number.format(site.budget)}</span
                        >
                      </div>
                      <div
                        class="progress-track"
                        role="meter"
                        aria-label={`Intentos de ${site.host}`}
                        aria-valuenow={site.attempted}
                        aria-valuemin={0}
                        aria-valuemax={Math.max(site.budget, site.attempted, 1)}
                      >
                        <span style:width={`${progress(site)}%`}></span>
                      </div></td
                    ></tr
                  >
                {:else}
                  <tr
                    ><td colspan="5" class="table-empty"
                      ><strong
                        >{snapshot
                          ? "No hay sitios con este filtro"
                          : loading
                            ? "Consultando el crawler…"
                            : "Esperando al crawler"}</strong
                      ><span
                        >{snapshot
                          ? "Prueba otro estado o término de búsqueda."
                          : "Los sitios aparecerán cuando el proceso publique su estado."}</span
                      ></td
                    ></tr
                  >
                {/each}
              </tbody>
            </table>
          </div>
          <div class="table-footer">
            <span
              >{filteredSites.length} de {snapshot?.sites.length ?? 0} sitios recientes</span
            ><span
              >Selecciona un sitio para inspeccionarlo <span aria-hidden="true"
                >→</span
              ></span
            >
          </div>
        </div>
        <aside class="inspector" aria-label="Detalle del sitio seleccionado">
          <div class="inspector-label">INSPECCIÓN DEL SITIO</div>
          {#if selected}
            <h3>{selected.host}</h3>
            <span class="detail-status"
              >{labels[selected.status]} · {time(
                selected.last_activity_at,
              )}</span
            >
            <div class="detail-url">
              <span class="field-label">URL ACTUAL</span
              >{#if selected.current_url}<a
                  href={safeLink(selected.current_url)}
                  target="_blank"
                  rel="noreferrer"
                  >{selected.current_url}<span aria-hidden="true"> ↗</span></a
                >{:else}<p>
                  {["completed", "stopped", "failed", "deferred", "cancelled", "storage_failed"].includes(selected.status)
                    ? "Sin solicitud en curso."
                    : "Esperando la próxima solicitud."}
                </p>{/if}
            </div>
            <dl class="detail-list">
              <div>
                <dt>Última respuesta</dt>
                <dd>
                  {selected.last_http_status
                    ? `HTTP ${selected.last_http_status}`
                    : "—"}
                </dd>
              </div>
              <div>
                <dt>Duración de descarga</dt>
                <dd>
                  {selected.last_fetch_ms === null
                    ? "—"
                    : `${number.format(selected.last_fetch_ms)} ms`}
                </dd>
              </div>
              <div>
                <dt>Contenido guardado</dt>
                <dd>{bytes(selected.bytes)}</dd>
              </div>
              <div>
                <dt>Páginas omitidas</dt>
                <dd>{selected.skipped}</dd>
              </div>
              <div>
                <dt>HTTP de páginas y sitemaps</dt>
                <dd>{optionalCount(selected.page_http_attempts)}</dd>
              </div>
              <div>
                <dt>Saltos de redirección</dt>
                <dd>{optionalCount(selected.redirect_hops)}</dd>
              </div>
              <div>
                <dt>Páginas útiles durables</dt>
                <dd>{optionalCount(selected.useful_pages)}</dd>
              </div>
              <div>
                <dt>Aplazadas</dt>
                <dd>{optionalCount(selected.deferred)}</dd>
              </div>
              <div>
                <dt>Canceladas</dt>
                <dd>{optionalCount(selected.cancelled)}</dd>
              </div>
              <div>
                <dt>Fallos de almacenamiento</dt>
                <dd>{optionalCount(selected.storage_failed)}</dd>
              </div>
              <div>
                <dt>Próxima solicitud</dt>
                <dd>{time(selected.next_request_at)}</dd>
              </div>
            </dl>
            {#if selected.last_error}<div class="last-error">
                <span class="field-label">ÚLTIMA INCIDENCIA</span>
                <p>{selected.last_error}</p>
              </div>{/if}
          {:else}<p class="inspector-empty">
              El detalle estará disponible cuando haya sitios en la ejecución.
            </p>{/if}
        </aside>
      </div>
      <p class="section-note">
        El detalle conserva hasta 500 sitios recientes. Los contadores acumulan
        toda la ejecución.
      </p>
    </section>

    <div class="lower-grid">
      <section
        id="actividad"
        class="activity-section panel"
        aria-labelledby="activity-heading"
      >
        <div class="section-heading">
          <div class="section-title">
            <h2 id="activity-heading">Actividad reciente</h2>
            <span class="count">{events.length}</span>
          </div>
          <label class="event-filter"
            ><span class="sr-only">Filtrar actividad</span><select
              bind:value={eventFilter}
              ><option value="all">Toda la actividad</option><option
                value="errors">Incidencias</option
              ><option value="site">Sitio seleccionado</option></select
            ></label
          >
        </div>
        <div class="event-list">
          {#each events as event}<div
              class="event-row"
              class:event-error={event.level === "error"}
            >
              <time datetime={event.at}>{time(event.at)}</time><span
                class="event-mark"
                aria-hidden="true"
                >{event.level === "error"
                  ? "!"
                  : event.kind === "saved"
                    ? "↓"
                    : "·"}</span
              >
              <div class="event-content">
                <p>
                  {event.kind === "redirect"
                    ? `Redirección HTTP ${event.http_status ?? ""}`
                    : event.message}
                </p>
                {#if event.url}<a
                    href={safeLink(event.url)}
                    target="_blank"
                    rel="noreferrer"
                    title={event.url}>{event.url}</a
                  >{:else if event.host}<span class="event-host"
                    >{event.host}</span
                  >{/if}
              </div>
              <span class="event-code">{event.http_status ?? ""}</span>
            </div>{:else}<div class="events-empty">
              {snapshot
                ? "No hay eventos para este filtro."
                : "Aún no hay actividad disponible."}
            </div>{/each}
        </div>
        <div class="section-note">
          Últimos {snapshot?.recent_events.length ?? 0} eventos registrados · más
          recientes primero
        </div>
      </section>
      <section
        id="almacenamiento"
        class="storage-section panel"
        aria-labelledby="storage-heading"
      >
        <div class="section-heading">
          <h2 id="storage-heading">Archivos WARC</h2>
          <span class="storage-tag">DISCO LOCAL</span>
        </div>
        <div class="storage-totals">
          <div>
            <strong
              >{snapshot ? number.format(snapshot.warc.files) : "—"}</strong
            ><span>archivos</span>
          </div>
          <div>
            <strong>{bytes(snapshot?.warc.bytes)}</strong><span
              >almacenados</span
            >
          </div>
        </div>
        <div class="storage-field">
          <span class="field-label">DIRECTORIO DE SALIDA</span><code
            >{snapshot?.warc.directory ?? "Sin estado disponible"}</code
          >
        </div>
        <div class="storage-field">
          <span class="field-label">ÚLTIMO ARCHIVO REGISTRADO</span><code
            >{snapshot?.warc.current_file ??
              "Todavía no se han escrito archivos."}</code
          >
        </div>
        <div class="storage-foot">
          <span class="square-dot"></span> Descargas guardadas por el proceso crawler
        </div>
      </section>
    </div>
    <BenchmarkPanel
      onavailability={(available) => (benchmarkAvailable = available)}
    />
    <footer class="page-footer">
      <span
        >Astian crawler <span class="slash">/</span>
        {snapshot?.run_id ?? "Sin ejecución"}</span
      ><span>Hora de Bogotá · Estado: {time(snapshot?.updated_at)}</span>
    </footer>
  </main>
</div>
