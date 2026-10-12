import assert from "node:assert/strict";
import { mkdir, writeFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const { chromium } = await import(
  process.env.CRAWLER_PLAYWRIGHT_MODULE ||
    new URL("../../../frontend/node_modules/playwright/index.mjs", import.meta.url).href
);
const base = process.env.CRAWLER_LIVE_URL || "http://127.0.0.1:3100/";
const expectTransport = process.env.CRAWLER_EXPECT_TRANSPORT === "1";
const output = process.env.CRAWLER_LIVE_OUTPUT || join(root, expectTransport ? "data/crawler-c04-dashboard" : "data/crawler-c03-dashboard");
const endpoint = (path) => new URL(path, base);
const health = await fetch(endpoint("/healthz"));
assert.equal(health.status, 200);
const initialResponse = await fetch(endpoint("/api/status"));
assert.equal(initialResponse.status, 200);
const initial = await initialResponse.json();
assert.equal(initial.meta.process_alive, true);
assert.ok(initial.snapshot.telemetry, "the operational crawler must publish C03 telemetry");
if (expectTransport) assert.ok(initial.snapshot.transport, "the operational crawler must publish C04 transport measurements");
if (process.env.CRAWLER_EXPECTED_PID)
  assert.equal(initial.snapshot.pid, Number(process.env.CRAWLER_EXPECTED_PID));
const benchmark = await fetch(endpoint("/api/benchmarks"));
assert.equal(benchmark.status, 404);
assert.equal((await benchmark.json()).code, "BENCHMARK_DISABLED");
await mkdir(dirname(output), { recursive: true });

const browser = await chromium.launch({
  headless: true,
  executablePath: process.env.CHROMIUM_PATH || "/usr/bin/chromium",
  args: ["--no-sandbox"],
});
try {
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  const errors = [];
  const blockedRequests = [];
  page.on("pageerror", (error) => errors.push(String(error)));
  await page.route("**/*", (route) => {
    if (new URL(route.request().url()).origin === new URL(base).origin) return route.continue();
    blockedRequests.push(route.request().url());
    return route.abort();
  });
  await page.goto(base);
  await page.locator(".page-footer").filter({ hasText: initial.snapshot.run_id }).waitFor();
  await page.locator(".telemetry-panel > summary").click();
  if (expectTransport) await page.locator(".transport-panel > summary").click();
  await page.getByRole("button", { name: "Pausar lectura", exact: true }).click();
  const refreshing = page.waitForResponse((response) => new URL(response.url()).pathname === "/api/status");
  await page.getByRole("button", { name: "Actualizar estado ahora", exact: true }).click();
  const rendered = await (await refreshing).json();
  const stats = rendered.snapshot.telemetry;
  const formatter = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 1 });
  const telemetryValue = (label) => page.locator(".telemetry-details dl > div")
    .filter({ has: page.getByText(label, { exact: true }) }).locator("dd");
  for (const [label, key] of [
    ["Actualizaciones recibidas", "updates"],
    ["Publicaciones atómicas", "snapshot_writes"],
    ["Llamadas de escritura", "write_calls"],
    ["Eventos recibidos", "events_total"],
    ["Eventos fuera del buffer", "events_dropped"],
    ["Errores de publicación", "errors"],
  ]) {
    assert.equal(typeof stats[key], "number");
    await telemetryValue(label).filter({ hasText: formatter.format(stats[key]) }).waitFor();
    assert.equal(await telemetryValue(label).textContent(), formatter.format(stats[key]));
  }
  assert.equal(stats.events_total, stats.events_dropped + rendered.snapshot.recent_events.length);
  if (expectTransport) {
    const transport = rendered.snapshot.transport;
    const transportValue = (label) => page.locator(".transport-groups dl > div")
      .filter({ has: page.getByText(label, { exact: true }) }).locator("dd");
    for (const [label, key] of [
      ["Peticiones iniciadas", "requests_started"],
      ["Clientes activos", "client_slots_active"],
      ["Clientes disponibles", "client_slots_idle"],
      ["Clientes en el pool", "client_slots_total"],
      ["Límite global de clientes", "max_client_slots"],
      ["Límite de clientes por origen", "max_slots_per_origin"],
      ["Clientes creados", "clients_created"],
      ["Reutilizaciones de cliente", "clients_reused"],
      ["Consultas DNS", "dns_lookups"],
      ["Aciertos de caché DNS", "dns_cache_hits"],
      ["Aciertos de caché negativa", "dns_negative_hits"],
      ["Consultas simultáneas agrupadas", "dns_coalesced"],
      ["Destinos rechazados", "dns_rejected"],
      ["Consultas DNS lógicas en curso", "dns_inflight"],
      ["Timeouts de petición", "request_timeouts"],
      ["Timeouts de cuerpo", "body_timeouts"],
    ]) {
      assert.equal(typeof transport[key], "number");
      await transportValue(label).filter({ hasText: formatter.format(transport[key]) }).waitFor();
      assert.equal(await transportValue(label).textContent(), formatter.format(transport[key]));
    }
    for (const [label, key] of [
      ["Consultas DNS al sistema en curso", "dns_blocking_inflight"],
      ["Pico de consultas DNS al sistema", "dns_blocking_inflight_peak"],
    ]) {
      const expected = transport[key] == null ? "—" : formatter.format(transport[key]);
      await transportValue(label).filter({ hasText: expected }).waitFor();
      assert.equal(await transportValue(label).textContent(), expected);
    }
    assert.equal(transport.client_slots_active + transport.client_slots_idle, transport.client_slots_total);
    assert.ok(transport.client_slots_total <= transport.max_client_slots);
    assert.match(await page.locator(".transport-details > p").first().textContent(), /no miden conexiones TCP/);
  }
  const visibleAgeMs = Math.max(0, Date.now() - Date.parse(stats.last_published_at));
  assert.ok(visibleAgeMs <= 5000, `visible telemetry is ${visibleAgeMs} ms old`);
  assert.equal(await page.locator("#banco-c01").count(), 0);
  const desktopOverflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
  assert.equal(desktopOverflow, false);
  await page.getByRole("button", { name: "Reanudar lectura", exact: true }).click();
  await page.waitForTimeout(200);
  await page.screenshot({ path: `${output}-desktop.png`, fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  const mobileOverflow = await page.evaluate(() => document.documentElement.scrollWidth > window.innerWidth);
  assert.equal(mobileOverflow, false);
  await page.screenshot({ path: `${output}-mobile.png`, fullPage: true });
  assert.deepEqual(errors, []);
  assert.deepEqual(blockedRequests, []);
  const final = await (await fetch(endpoint("/api/status"))).json();
  assert.equal(final.meta.process_alive, true);
  assert.equal(final.snapshot.run_id, initial.snapshot.run_id);
  const evidence = {
    passed: true, checked_at: new Date().toISOString(), url: base,
    crawler_pid: final.snapshot.pid, run_id: final.snapshot.run_id,
    health_status: health.status, api_status: initialResponse.status,
    process_alive: final.meta.process_alive, phase: final.snapshot.phase,
    telemetry_verified: stats, visible_age_ms: visibleAgeMs,
    ...(expectTransport ? { transport_verified: rendered.snapshot.transport } : {}),
    counters_verified: rendered.snapshot.counters,
    retained_events: rendered.snapshot.recent_events.length,
    benchmark_disabled: true, fixtures_visible: false,
    desktop: { width: 1440, overflow: desktopOverflow, screenshot: `${output}-desktop.png` },
    mobile: { width: 390, overflow: mobileOverflow, screenshot: `${output}-mobile.png` },
    page_errors: errors, blocked_requests: blockedRequests,
    source: "Operational crawler snapshot; no fixtures or generated metrics.",
  };
  await writeFile(`${output}-verification.json`, JSON.stringify(evidence, null, 2) + "\n");
  console.log(JSON.stringify(evidence));
} finally {
  await browser.close();
}
