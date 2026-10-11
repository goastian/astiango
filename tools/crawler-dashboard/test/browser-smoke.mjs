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
  assert.deepEqual(errors, []);
  console.log(
    JSON.stringify({
      passed: true,
      productionCounter: 91,
      benchmarkRows: 2,
      benchmarkVisibleWithoutProduction: true,
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
