<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { Snapshot } from "./types";
  let { snapshot }: { snapshot?: Snapshot | null } = $props();
  const formatter = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 2 });
  const number = (value?: number) => value === undefined ? "—" : formatter.format(value);
  const scheduler = $derived(snapshot?.scheduler);
  const robots = $derived(snapshot?.robots);
  const frontier = $derived(snapshot?.frontier_db);
</script>
{#if scheduler || robots}
  <details class="panel scheduling-panel">
    <summary><strong>Identidad, turnos y robots por origen</strong>
      <span class="summary">{scheduler?.mode === "fair_turns" ? "Turnos cortos · otros orígenes durante las pausas" : "Jobs largos"}</span>
      <span class="toggle">Detalles <Icon name="chevron" size={16} /></span>
    </summary>
    <div class="content"><p>Cada blog o tenant de un sufijo privado conserva su presupuesto. Las URLs mantienen sus parámetros y rutas.
      Robots, páginas y sitemaps comparten la cortesía del origen: esquema, host y puerto.</p>
      <div class="groups">
        <section aria-label="Identidad y migración"><h3>Identidad y migración</h3><dl>
          <div><dt>Versión de URL / sitio</dt><dd>{number(frontier?.identity_version)} / {number(frontier?.site_identity_version)}</dd></div>
          <div><dt>Sitios independientes conservados</dt><dd>{number(frontier?.known_sites)}</dd></div>
          <div><dt>Correspondencias de claves migradas</dt><dd>{number(frontier?.identity_mapped_urls)}</dd></div>
          <div><dt>Migración de identidad</dt><dd>{frontier?.identity_migration_complete ? "Completada" : "Pendiente"}</dd></div>
        </dl></section>
        <section aria-label="Admisión y cortesía"><h3>Admisión y cortesía</h3><dl>
          <div><dt>Solicitudes admitidas</dt><dd>{number(scheduler?.requests_admitted)}</dd></div>
          <div><dt>Admisiones aplazadas</dt><dd>{number(scheduler?.requests_deferred)}</dd></div>
          <div><dt>Solicitudes activas / pico</dt><dd>{number(scheduler?.active_requests)} / {number(scheduler?.active_peak)}</dd></div>
          <div><dt>Orígenes controlados / cota</dt><dd>{number(scheduler?.tracked_origins)} / {number(scheduler?.origin_limit)}</dd></div>
          <div><dt>Máximo activo por sitio / IP</dt><dd>{number(scheduler?.site_limit)} / {number(snapshot?.transport?.max_active_per_ip)}</dd></div>
          <div><dt>Reservas de IP / esperas</dt><dd>{number(snapshot?.transport?.active_ip_reservations)} / {number(snapshot?.transport?.ip_waits)}</dd></div>
          <div><dt>Desbordes de admisión</dt><dd>{number(scheduler?.origin_overflows)}</dd></div>
        </dl></section>
        <section aria-label="Robots por origen"><h3>Robots por origen</h3><dl>
          <div><dt>Aciertos de caché / descargas</dt><dd>{number(robots?.hits)} / {number(robots?.downloads)}</dd></div>
          <div><dt>Consultas simultáneas agrupadas</dt><dd>{number(robots?.single_flight_waits)}</dd></div>
          <div><dt>Entradas / cota</dt><dd>{number(robots?.entries)} / {number(robots?.capacity)}</dd></div>
          <div><dt>TTL / revalidación de fallos · s</dt><dd>{number(robots?.ttl_seconds)} / {number(robots?.failure_ttl_seconds)}</dd></div>
          <div><dt>Fallos temporales / rutas excluidas</dt><dd>{number(robots?.unreachable)} / {number(robots?.disallowed)}</dd></div>
        </dl></section>
      </div>
    </div>
  </details>
{/if}
<style>
  .scheduling-panel { margin-top: 20px; }
  summary { display: flex; align-items: center; gap: 16px; padding: 20px 24px; cursor: pointer; list-style: none; }
  summary::-webkit-details-marker { display: none; }
  summary strong, h3 { font-size: 12px; }
  .summary, .toggle { font-size: 11px; color: #7d8799; }
  .toggle { margin-left: auto; display: flex; align-items: center; gap: 8px; }
  .content { padding: 0 24px 20px; }
  .groups { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 24px; }
  p { font-size: 12px; color: #697386; line-height: 1.6; }
  dl div { display: flex; justify-content: space-between; gap: 16px; padding: 8px 0; border-bottom: 1px solid #eef0f4; font-size: 12px; }
  dd { margin: 0; text-align: right; }
  @media (max-width: 900px) { summary { flex-wrap: wrap; gap: 10px; } .groups { grid-template-columns: 1fr; } .content { padding: 0 16px 16px; } }
</style>
