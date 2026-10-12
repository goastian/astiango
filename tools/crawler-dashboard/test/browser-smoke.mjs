import assert from "node:assert/strict";
import { mkdtemp, writeFile, rm, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { createDashboardServer } from "../server.mjs";
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
      benchmarkObserverModes: ["legacy_sync", "async"],
      benchmarkBehaviorDifferencesVisible: true,
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
