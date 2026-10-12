import assert from "node:assert/strict";
import { mkdtemp, writeFile, rm, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createDashboardServer } from "../server.mjs";
import { transportReport } from "./transport-report.mjs";
import { bodyReport, bodyStats } from "./body-fixture.mjs";
// Reuse the repository's existing browser-test dependency; no network install.
const { chromium } = await import(
  process.env.CRAWLER_PLAYWRIGHT_MODULE ||
    new URL(
      "../../../frontend/node_modules/playwright/index.mjs",
      import.meta.url,
    ).href
);
const directory = await mkdtemp(join(tmpdir(), "c01-browser-"));
const statePath = join(directory, "status.json");
const benchmarkPath = join(directory, "report.json");
const metric = { median: 12, min: 10, max: 14, mad: 2 };
const now = new Date().toISOString();
const snapshot = {
  schema_version: 1,
  run_id: "production-kept",
  pid: process.pid,
  started_at: now,
  updated_at: now,
  finished_at: now,
  phase: "completed",
  user_agent: "Test",
  workers: 2,
  min_crawl_delay_ms: 100,
  pages_per_domain: 5,
  counters: {
    downloaded: 91,
    failed: 0,
    skipped: 0,
    attempted: 91,
    bytes: 1024,
    sites: 0,
    completed_sites: 0,
  },
  sites: [],
  recent_events: [],
  warc: { directory: "production", files: 2, bytes: 1024, current_file: null },
  history: [],
};
const report = {
  schema_version: 1,
  kind: "crawler-capacity-benchmark",
  generated_at: now,
  configuration: { repetitions: 3, origins: [2, 32, 128] },
  code: {
    commit: "test-commit",
    dirty: true,
    source_sha256: "test-source-hash",
  },
  corpus: { sha256: "test-corpus-hash", files: ["article.html"] },
  hardware: { cpu_count: 8 },
  scenarios: [false, true].map((observer) => ({
    name: "adversarial",
    origins: 128,
    observer,
    repetitions: 3,
    metrics: {
      duration_ms: metric,
      cpu_seconds: metric,
      rss_peak_bytes: metric,
      requests: metric,
      saved_pages: metric,
      write_bytes: metric,
      read_bytes: metric,
      syscw: metric,
      fds_peak: metric,
      threads_peak: metric,
      saved_bytes: metric,
      stages_ms: {
        dns: metric,
        http_headers: metric,
        politeness: metric,
        sink: metric,
        queue_wait: metric,
      },
    },
    requests_match: true,
    ...(observer
      ? {
          runs: [
            {
              repetition: 1,
              errors: { other: 1, http_404: 2 },
              request_failures: 0,
              saved_pages: 11,
              client_requests: 12,
              server: { requests: 12 },
            },
            {
              repetition: 2,
              errors: {},
              request_failures: 0,
              saved_pages: 12,
              client_requests: 12,
              server: { requests: 12 },
            },
            {
              repetition: 3,
              errors: {},
              request_failures: 0,
              saved_pages: 12,
              client_requests: 12,
              server: { requests: 12 },
            },
          ],
        }
      : {}),
  })),
  comparisons: [
    {
      name: "adversarial",
      origins: 128,
      duration_overhead_pct: 2.3,
      requests_equal: true,
      saved_pages_equal: true,
    },
  ],
  acceptance: { passed: true },
};
await writeFile(statePath, JSON.stringify(snapshot));
await writeFile(benchmarkPath, JSON.stringify(report));
const servers = [];
const browser = await chromium.launch({
  headless: true,
  executablePath: process.env.CHROMIUM_PATH || "/usr/bin/chromium",
  args: ["--no-sandbox"],
});
try {
  const page = await browser.newPage({
    viewport: { width: 1440, height: 1000 },
  });
  const errors = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.route("**/*", (route) =>
    new URL(route.request().url()).hostname === "127.0.0.1"
      ? route.continue()
      : route.abort(),
  );
  for (const mode of ["disabled", "alongside-production", "benchmark-only"]) {
    const enabled = mode !== "disabled";
    const server = createDashboardServer({
      statePath:
        mode === "benchmark-only" ? join(directory, "missing.json") : statePath,
      benchmarkPath: enabled ? benchmarkPath : null,
    });
    servers.push(server);
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    await page.goto(`http://127.0.0.1:${server.address().port}`);
    if (mode === "benchmark-only")
      await page
        .getByText("El crawler todavía no ha publicado su estado.", {
          exact: false,
        })
        .waitFor();
    else
      await page
        .locator(".page-footer")
        .filter({ hasText: "production-kept" })
        .waitFor();
    if (!enabled) {
      await page.waitForTimeout(400);
      assert.equal(await page.locator("#banco-c01").count(), 0);
      assert.deepEqual(
        await page.locator(".request-metrics dd").allTextContents(),
        ["—", "—", "—"],
        "legacy snapshots do not fabricate C02 counters",
      );
      assert.equal(await page.locator(".telemetry-panel").count(), 0);
      assert.equal(await page.locator(".transport-panel").count(), 0);
      assert.equal(await page.locator(".body-panel").count(), 0);
    } else {
      await page.getByRole("link", { name: "Banco C01 · local" }).click();
      await page.getByText("Aceptación verificada", { exact: true }).waitFor();
      assert.equal(
        await page
          .locator("#banco-c01 .benchmark-table")
          .first()
          .locator("tbody tr")
          .count(),
        2,
      );
      await page
        .getByRole("combobox", { name: "Escenario para inspeccionar etapas" })
        .selectOption("1");
      const runTable = page.getByRole("table", {
        name: "Errores por repetición",
        exact: true,
      });
      assert.equal(await runTable.locator("tbody tr").count(), 3);
      assert.match(
        await runTable.locator("tbody tr").first().textContent(),
        /Otros errores: 1/,
      );
      assert.match(
        await runTable.locator("tbody tr").first().textContent(),
        /HTTP 404 · no encontrado: 2/,
      );
      assert.equal(
        (
          await runTable
            .locator("tbody tr")
            .first()
            .locator("td")
            .first()
            .textContent()
        ).trim(),
        "11",
      );
      await page
        .getByRole("combobox", { name: "Escenario para inspeccionar etapas" })
        .selectOption("0");
      await runTable
        .getByText("Sin detalle por repetición publicado.", { exact: true })
        .waitFor();
      await page
        .getByRole("combobox", { name: "Escenario para inspeccionar etapas" })
        .selectOption("1");
      await page
        .getByText(
          "Código, corpus, hardware y configuración de esta medición",
          { exact: true },
        )
        .click();
      await page.getByText("test-corpus-hash", { exact: true }).waitFor();
      await page
        .getByRole("rowheader", { name: "Escrituras de disco", exact: true })
        .waitFor();
      await page
        .getByRole("rowheader", {
          name: "Descriptores · pico muestreado",
          exact: true,
        })
        .waitFor();
      const screenshotDirectory = process.env.CRAWLER_SCREENSHOT_DIR;
      if (screenshotDirectory) {
        await mkdir(screenshotDirectory, { recursive: true });
        await page.locator("#banco-c01").screenshot({
          path: join(screenshotDirectory, `c01-${mode}-desktop.png`),
        });
      }
      await page.setViewportSize({ width: 390, height: 844 });
      await page.locator("#banco-c01").scrollIntoViewIfNeeded();
      assert.equal(
        await page.evaluate(
          () => document.documentElement.scrollWidth > window.innerWidth,
        ),
        false,
      );
      if (screenshotDirectory)
        await page.locator("#banco-c01").screenshot({
          path: join(screenshotDirectory, `c01-${mode}-mobile.png`),
        });
      assert.equal(
        (
          await page.locator(".pages-panel .metric-value strong").textContent()
        ).trim(),
        mode === "benchmark-only" ? "—" : "91",
      );
      const downloadEvent = page.waitForEvent("download");
      await page
        .getByRole("button", { name: "Descargar C01", exact: true })
        .click();
      assert.match((await downloadEvent).suggestedFilename(), /^crawler-c01-/);
      await page.setViewportSize({ width: 1440, height: 1000 });
    }
  }
  const c02Snapshot = structuredClone(snapshot);
  c02Snapshot.run_id = "c02-explicit-outcomes";
  Object.assign(c02Snapshot.counters, {
    failed: 2, skipped: 3, page_http_attempts: 107, redirect_hops: 8,
    useful_pages: 91, deferred: 4, cancelled: 1, storage_failed: 2,
  });
  c02Snapshot.sites = ["deferred", "cancelled", "storage_failed"].map((status) => ({
    host: `${status}.test`, status, current_url: null, pages_downloaded: 0,
    failed: 0, skipped: 0, attempted: 1, budget: 5, bytes: 0,
    last_activity_at: now, last_http_status: null, last_error: "Motivo explícito",
    last_fetch_ms: null, next_request_at: null, page_http_attempts: 2,
    redirect_hops: 1, useful_pages: 0, deferred: 0, cancelled: 0, storage_failed: 0,
    [status]: 1,
  }));
  await writeFile(statePath, JSON.stringify(c02Snapshot));
  const c02Server = createDashboardServer({ statePath });
  servers.push(c02Server);
  await new Promise((resolve) => c02Server.listen(0, "127.0.0.1", resolve));
  await page.goto(`http://127.0.0.1:${c02Server.address().port}`);
  await page.locator(".page-footer").filter({ hasText: "c02-explicit-outcomes" }).waitFor();
  assert.deepEqual(await page.locator(".request-metrics dd").allTextContents(), ["107", "8", "91"]);
  assert.deepEqual(await page.locator(".explicit-outcomes dd").allTextContents(), ["3", "2", "4", "1", "2"]);
  await page.getByRole("button", { name: "Con incidencias", exact: true }).click();
  assert.equal(await page.locator(".sites-table tbody tr").count(), 3);
  await page.getByRole("button", { name: "storage_failed.test", exact: false }).first().click();
  await page.locator(".detail-status").filter({ hasText: "Fallo de almacenamiento" }).waitFor();
  assert.equal(await page.locator(".gauge-label strong").textContent(), "95,8%");
  const screenshotDirectory = process.env.CRAWLER_SCREENSHOT_DIR;
  if (screenshotDirectory) await page.screenshot({ path: join(screenshotDirectory, "c02-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  if (screenshotDirectory) await page.screenshot({ path: join(screenshotDirectory, "c02-mobile.png"), fullPage: true });
  const c03Snapshot = structuredClone(snapshot);
  c03Snapshot.run_id = "c03-observer-health";
  c03Snapshot.phase = "running";
  c03Snapshot.finished_at = null;
  c03Snapshot.updated_at = new Date(Date.now() - 60_000).toISOString();
  c03Snapshot.telemetry = {
    projection_interval_ms: 1000, event_capacity: 100, updates: 240,
    events_total: 250, events_dropped: 150, snapshot_writes: 20,
    write_calls: 22, bytes_written: 2048, errors: 3, pending_updates: 4,
    last_error: null, last_failure_at: c03Snapshot.updated_at,
    last_published_at: new Date().toISOString(), last_publish_latency_ms: 30,
    max_publish_latency_ms: 70, legacy_write_estimate: 241,
  };
  c03Snapshot.recent_events = Array.from({ length: 100 }, (_, index) => ({
    at: now, level: "info", kind: "saved", message: `Página conservada ${index}`,
    host: "example.test", url: null, http_status: 200, bytes: 1,
    fetch_time_ms: 1, warc_file: null,
  }));
  await writeFile(statePath, JSON.stringify(c03Snapshot));
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(`http://127.0.0.1:${c02Server.address().port}`);
  await page.locator(".page-footer").filter({ hasText: "c03-observer-health" }).waitFor();
  await page.locator(".telemetry-health").filter({ hasText: "Publicación al día" }).waitFor();
  assert.equal((await page.locator(".connection").textContent()).trim(), "En ejecución");
  await page.locator(".telemetry-panel > summary").click();
  const telemetryValue = (label) => page.locator(".telemetry-details dl > div").filter({ has: page.getByText(label, { exact: true }) }).locator("dd");
  assert.equal(await telemetryValue("Publicaciones atómicas").textContent(), "20");
  assert.equal(await telemetryValue("Llamadas de escritura").textContent(), "22");
  assert.equal(await telemetryValue("Demora de última publicación").textContent(), "30 ms");
  assert.equal(await telemetryValue("Eventos recibidos").textContent(), "250");
  assert.equal(await telemetryValue("Eventos fuera del buffer").textContent(), "150");
  assert.equal(await telemetryValue("Errores de publicación").textContent(), "3");
  assert.match(await page.locator(".activity-section .section-note").textContent(), /100 eventos registrados.*150 fuera del buffer/s);
  c03Snapshot.telemetry.last_error = "controlled metrics output failure";
  await writeFile(statePath, JSON.stringify(c03Snapshot));
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  await page.locator(".telemetry-health").filter({ hasText: "Salida de métricas degradada" }).waitFor();
  assert.equal((await page.locator(".connection").textContent()).trim(), "En ejecución");
  assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(), "91");
  if (screenshotDirectory) await page.locator(".telemetry-panel").screenshot({ path: join(screenshotDirectory, "c03-degraded.png") });
  c03Snapshot.telemetry.last_error = null;
  c03Snapshot.telemetry.last_published_at = c03Snapshot.updated_at;
  await writeFile(statePath, JSON.stringify(c03Snapshot));
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  await page.locator(".connection").filter({ hasText: "Telemetría sin actualizar" }).waitFor();
  await page.locator(".telemetry-health").filter({ hasText: "Publicación sin actualizar" }).waitFor();
  await writeFile(statePath, "{broken");
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  await page.locator(".telemetry-health").filter({ hasText: "Lectura no disponible" }).waitFor();
  assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(), "91");
  c03Snapshot.telemetry.last_published_at = new Date().toISOString();
  await writeFile(statePath, JSON.stringify(c03Snapshot));
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  await page.locator(".telemetry-health").filter({ hasText: "Publicación al día" }).waitFor();
  if (screenshotDirectory) await page.screenshot({ path: join(screenshotDirectory, "c03-desktop.png"), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  if (screenshotDirectory) await page.screenshot({ path: join(screenshotDirectory, "c03-mobile.png"), fullPage: true });
  const c04Snapshot = structuredClone(c03Snapshot);
  c04Snapshot.run_id = "c04-http-dns";
  c04Snapshot.telemetry.last_published_at = new Date().toISOString();
  c04Snapshot.transport = {
    requests_started: 120, headers_received: 116, request_failures: 4,
    request_timeouts: 2, body_timeouts: 1, max_client_slots: 128, max_slots_per_origin: 4,
    bodies_completed: 100, body_failures: 3, bodies_dropped: 11, bytes_received: 98304,
    client_slots_total: 8, client_slots_active: 2, client_slots_idle: 6,
    client_slots_peak: 10, clients_created: 12, clients_reused: 108,
    clients_evicted: 4, pool_waits: 7, pool_wait_ms: 230,
    dns_cache_entries: 5, dns_cache_hits: 110, dns_negative_hits: 2,
    dns_cache_misses: 8, dns_coalesced: 3, dns_lookups: 5, dns_failures: 1,
    dns_rejected: 2, dns_inflight: 0, dns_inflight_peak: 3, dns_lookup_ms: 50,
  };
  await writeFile(statePath, JSON.stringify(c04Snapshot));
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  await page.locator(".page-footer").filter({ hasText: "c04-http-dns" }).waitFor();
  await page.locator(".transport-panel > summary").click();
  const transportValue = (label) => page.locator(".transport-groups dl > div")
    .filter({ has: page.getByText(label, { exact: true }) }).locator("dd");
  for (const [label, expected] of [
    ["Peticiones iniciadas", "120"], ["Reutilizaciones de cliente", "108"],
    ["Clientes activos", "2"], ["Clientes disponibles", "6"],
    ["Límite global de clientes", "128"], ["Límite de clientes por origen", "4"],
    ["Timeouts de petición", "2"], ["Timeouts de cuerpo", "1"],
    ["Timeouts de conexión", "—"], ["Timeouts de cabeceras", "—"], ["Timeouts totales antes del cuerpo", "—"],
    ["Consultas DNS", "5"], ["Aciertos de caché DNS", "110"],
    ["Aciertos de caché negativa", "2"], ["Consultas simultáneas agrupadas", "3"],
    ["Destinos rechazados", "2"], ["Consultas DNS lógicas en curso", "0"],
    ["Consultas DNS al sistema en curso", "—"], ["Pico de consultas DNS al sistema", "—"],
    ["Espera acumulada del pool", "230 ms"], ["Cuerpos liberados antes de terminar", "11"],
  ]) assert.equal(await transportValue(label).textContent(), expected);
  Object.assign(c04Snapshot.transport, { dns_blocking_inflight: 1, dns_blocking_inflight_peak: 2 });
  await writeFile(statePath, JSON.stringify(c04Snapshot));
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  await transportValue("Consultas DNS al sistema en curso").filter({ hasText: "1" }).waitFor();
  assert.equal(await transportValue("Consultas DNS lógicas en curso").textContent(), "0");
  assert.equal(await transportValue("Pico de consultas DNS al sistema").textContent(), "2");
  assert.match(await page.locator(".transport-details > p").first().textContent(), /no miden conexiones TCP/);
  assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(), "91");
  await writeFile(statePath, "{broken");
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  await page.locator(".transport-notice").waitFor();
  assert.equal(await transportValue("Reutilizaciones de cliente").textContent(), "108");
  c04Snapshot.telemetry.last_published_at = new Date().toISOString();
  await writeFile(statePath, JSON.stringify(c04Snapshot));
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  await page.locator(".transport-notice").waitFor({ state: "hidden" });
  if (screenshotDirectory) await page.locator(".transport-panel").screenshot({ path: join(screenshotDirectory, "c04-transport-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  if (screenshotDirectory) await page.locator(".transport-panel").screenshot({ path: join(screenshotDirectory, "c04-transport-mobile.png") });
  const c05Snapshot = structuredClone(c04Snapshot);
  c05Snapshot.run_id = "c05-admission-bodies";
  c05Snapshot.telemetry.last_published_at = new Date().toISOString();
  c05Snapshot.body = bodyStats();
  Object.assign(c05Snapshot.transport, { connect_timeouts: 1, header_timeouts: 1, total_timeouts: 0 });
  await writeFile(statePath, JSON.stringify(c05Snapshot));
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  await page.locator(".page-footer").filter({ hasText: "c05-admission-bodies" }).waitFor();
  await page.locator(".body-panel > summary").click();
  const bodyValue = (label) => page.locator(".body-groups dl > div")
    .filter({ has: page.getByText(label, { exact: true }) }).locator("dd");
  for (const [label, expected] of [
    ["Rechazos por status", "2"], ["Rechazos por MIME", "3"],
    ["Rechazos por Content-Length", "1"], ["Codificación no admitida", "4"],
    ["Bytes de cuerpo recibidos", "9.437.184 B"], ["Bytes de texto UTF-8 admitidos", "8.388.608 B"],
    ["Límite de recepción excedido", "2"], ["Límite de texto excedido", "1"],
    ["Timeouts por inactividad", "2"], ["Timeouts de duración total", "2"],
    ["Timeouts esperando presupuesto", "1"], ["Cuerpos con reserva activa", "1"],
    ["Bytes reservados actuales", "20.971.520 B"], ["Límite de bytes reservados", "25.165.824 B"],
    ["Espera acumulada de presupuesto", "340 ms"], ["Fallos al reservar memoria", "1"],
  ]) assert.equal(await bodyValue(label).textContent(), expected);
  assert.equal(await transportValue("Timeouts de conexión").textContent(), "1");
  assert.match(await page.locator(".body-details > p").first().textContent(), /no representa toda la memoria/);
  assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(), "91");
  await writeFile(statePath, "{broken");
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  await page.locator(".body-notice").waitFor();
  assert.equal(await bodyValue("Rechazos por MIME").textContent(), "3");
  assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(), "91");
  c05Snapshot.telemetry.last_published_at = new Date().toISOString();
  await writeFile(statePath, JSON.stringify(c05Snapshot));
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  await page.locator(".body-notice").waitFor({ state: "hidden" });
  if (screenshotDirectory) await page.locator(".body-panel").screenshot({ path: join(screenshotDirectory, "c05-body-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  if (screenshotDirectory) await page.locator(".body-panel").screenshot({ path: join(screenshotDirectory, "c05-body-mobile.png") });
  const c03Report = structuredClone(report);
  c03Report.configuration.acceptance_profile = "c03";
  c03Report.configuration.observer_modes = ["legacy_sync", "async"];
  c03Report.scenarios.forEach((scenario, index) => {
    scenario.observer = true;
    scenario.observer_mode = index === 0 ? "legacy_sync" : "async";
    scenario.metrics.observer_stats = { write_calls: metric, updates: metric, max_publish_latency_ms: metric };
  });
  Object.assign(c03Report.comparisons[0], {
    behavior_equivalent: false,
    write_calls_reduction_pct: { median: -2, min: -3, max: -1, mad: 1 },
    writes_per_update_reduction_pct: { median: 95, min: 94, max: 96, mad: 1 },
  });
  await writeFile(benchmarkPath, JSON.stringify(c03Report));
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.goto(`http://127.0.0.1:${servers[1].address().port}`);
  await page.getByRole("link", { name: "Banco C03 · local", exact: true }).click();
  await page.getByRole("cell", { name: "Sincrónico legado", exact: true }).waitFor();
  await page.getByRole("cell", { name: "Asíncrono", exact: true }).waitFor();
  await page.locator(".benchmark-table thead th").filter({ hasText: /^Reducción por actualización$/ }).waitFor();
  await page.getByRole("cell", { name: "Decisiones o trabajo diferentes", exact: true }).waitFor();
  await page.getByRole("cell", { name: "No comparable", exact: true }).waitFor();
  await page.getByRole("combobox", { name: "Escenario para inspeccionar etapas" }).selectOption("1");
  await page.getByRole("table", { name: "Recursos del observador por escenario" }).waitFor();
  const c03Download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Descargar C03", exact: true }).click();
  assert.match((await c03Download).suggestedFilename(), /^crawler-c03-/);
  if (screenshotDirectory) await page.locator("#banco-c01").screenshot({ path: join(screenshotDirectory, "c03-benchmark.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  await writeFile(benchmarkPath, JSON.stringify(transportReport()));
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole("button", { name: "Actualizar banco", exact: true }).click();
  await page.getByRole("link", { name: "Banco C04 · local", exact: true }).waitFor();
  const transportTable = page.getByRole("table", { name: "Resultados del transporte C04", exact: true });
  assert.equal(await transportTable.locator("tbody tr").count(), 4);
  await page.getByRole("combobox", { name: "Escenario de transporte para inspeccionar", exact: true }).selectOption("1");
  const connectionsTable = page.getByRole("table", { name: "Conexiones reales por repetición", exact: true });
  assert.equal(await connectionsTable.locator("tbody tr").count(), 3);
  assert.equal(await connectionsTable.locator("tbody tr").first().locator("td").nth(2).textContent(), "2");
  assert.equal((await page.locator(".pages-panel .metric-value strong").textContent()).trim(), "91");
  const c04Download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Descargar C04", exact: true }).click();
  assert.match((await c04Download).suggestedFilename(), /^crawler-c04-/);
  if (screenshotDirectory) await page.locator("#banco-c01").screenshot({ path: join(screenshotDirectory, "c04-benchmark-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  if (screenshotDirectory) await page.locator("#banco-c01").screenshot({ path: join(screenshotDirectory, "c04-benchmark-mobile.png") });
  await writeFile(benchmarkPath, JSON.stringify(bodyReport()));
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.getByRole("button", { name: "Actualizar banco", exact: true }).click();
  await page.getByRole("link", { name: "Banco C05 · local", exact: true }).waitFor();
  const bodyTable = page.getByRole("table", { name: "Resultados de admisión y cuerpos C05", exact: true });
  assert.equal(await bodyTable.locator("tbody tr").count(), 3);
  await page.getByRole("combobox", { name: "Escenario de cuerpos para inspeccionar", exact: true }).selectOption("1");
  const bodyRuns = page.getByRole("table", { name: "Resultados de cuerpos por repetición", exact: true });
  assert.equal(await bodyRuns.locator("tbody tr").count(), 3);
  const firstBodyRun = bodyRuns.locator("tbody tr").first().locator("td");
  assert.match(await firstBodyRun.nth(0).textContent(), /rejected_mime/);
  assert.equal(await firstBodyRun.nth(1).textContent(), "0 B");
  assert.equal(await firstBodyRun.nth(3).textContent(), "32.768 B");
  assert.equal(await firstBodyRun.nth(4).textContent(), "Reglas cumplidas");
  const c05Download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Descargar C05", exact: true }).click();
  assert.match((await c05Download).suggestedFilename(), /^crawler-c05-/);
  if (screenshotDirectory) await page.locator("#banco-c01").screenshot({ path: join(screenshotDirectory, "c05-benchmark-desktop.png") });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.equal(await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth), false);
  if (screenshotDirectory) await page.locator("#banco-c01").screenshot({ path: join(screenshotDirectory, "c05-benchmark-mobile.png") });
  assert.deepEqual(errors, []);
  console.log(
    JSON.stringify({
      passed: true,
      productionCounter: 91,
      benchmarkRows: 2,
      benchmarkVisibleWithoutProduction: true,
      explicitOutcomes: true,
      legacyCountersRemainUnknown: true,
      telemetryHealthAndRecovery: true,
      telemetryDroppedEvents: 150,
      unreadableMetricsKeepLastSnapshot: true,
      transportMetricsCompatible: true,
      clientReuseIsNotTcpConnections: true,
      transportReadFailureKeepsCounters: true,
      logicalAndSystemDnsAreSeparate: true,
      legacySystemDnsCountersRemainUnknown: true,
      benchmarkObserverModes: ["legacy_sync", "async"],
      benchmarkBehaviorDifferencesVisible: true,
      benchmarkTransportModesAndProtocols: true,
      benchmarkActualServerConnections: true,
      bodyAdmissionCompatible: true,
      bodyReadFailureKeepsCounters: true,
      residentBudgetIsNotRss: true,
      benchmarkBodyBytesSeparated: true,
      expectedRejectionCanPassVerification: true,
      mobileOverflow: false,
      pageErrors: errors,
    }),
  );
} finally {
  await browser.close();
  for (const server of servers) {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
  await rm(directory, { recursive: true, force: true });
}
