<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import Icon from "./Icon.svelte";
  import type { TransportStats } from "./types";

  let { stats, readError, stale }: {
    stats?: TransportStats;
    readError: boolean;
    stale: boolean;
  } = $props();
  const number = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 1 }));
  const count = (value: number | undefined) => value == null ? "—" : number.format(value);
  const groups = $derived([
    { title: "Peticiones y cuerpos", metrics: [
      { label: "Peticiones iniciadas", value: stats?.requests_started },
      { label: "Cabeceras recibidas", value: stats?.headers_received },
      { label: "Fallos de petición", value: stats?.request_failures },
      { label: "Timeouts de petición", value: stats?.request_timeouts },
      { label: "Timeouts de conexión", value: stats?.connect_timeouts },
      { label: "Timeouts de cabeceras", value: stats?.header_timeouts },
      { label: "Timeouts totales antes del cuerpo", value: stats?.total_timeouts },
      { label: "Cuerpos completados", value: stats?.bodies_completed },
      { label: "Fallos de cuerpo", value: stats?.body_failures },
      { label: "Timeouts de cuerpo", value: stats?.body_timeouts },
      { label: "Cuerpos liberados antes de terminar", value: stats?.bodies_dropped },
      { label: "Bytes recibidos", value: stats?.bytes_received, unit: "B" },
    ] },
    { title: "Pool de clientes", metrics: [
      { label: "Clientes activos", value: stats?.client_slots_active },
      { label: "Clientes disponibles", value: stats?.client_slots_idle },
      { label: "Clientes en el pool", value: stats?.client_slots_total },
      { label: "Pico de clientes en el pool", value: stats?.client_slots_peak },
      { label: "Límite global de clientes", value: stats?.max_client_slots },
      { label: "Límite de clientes por origen", value: stats?.max_slots_per_origin },
      { label: "Clientes creados", value: stats?.clients_created },
      { label: "Reutilizaciones de cliente", value: stats?.clients_reused },
      { label: "Clientes retirados", value: stats?.clients_evicted },
      { label: "Esperas por un cliente", value: stats?.pool_waits },
      { label: "Espera acumulada del pool", value: stats?.pool_wait_ms, unit: "ms" },
    ] },
    { title: "Resolución DNS", metrics: [
      { label: "Consultas DNS", value: stats?.dns_lookups },
      { label: "Aciertos de caché DNS", value: stats?.dns_cache_hits },
      { label: "Aciertos de caché negativa", value: stats?.dns_negative_hits },
      { label: "Consultas sin caché", value: stats?.dns_cache_misses },
      { label: "Consultas simultáneas agrupadas", value: stats?.dns_coalesced },
      { label: "Entradas de caché DNS", value: stats?.dns_cache_entries },
      { label: "Fallos de resolución", value: stats?.dns_failures },
      { label: "Destinos rechazados", value: stats?.dns_rejected },
      { label: "Consultas DNS lógicas en curso", value: stats?.dns_inflight },
      { label: "Pico de consultas DNS lógicas", value: stats?.dns_inflight_peak },
      { label: "Consultas DNS al sistema en curso", value: stats?.dns_blocking_inflight },
      { label: "Pico de consultas DNS al sistema", value: stats?.dns_blocking_inflight_peak },
      { label: "Tiempo acumulado de resolución", value: stats?.dns_lookup_ms, unit: "ms" },
    ] },
  ]);
</script>

{#if stats}
  <details class="panel transport-panel">
    <summary>
      <span class="transport-heading">{$t("HTTP y DNS")}</span>
      <span class="transport-summary">{$t("{p0} peticiones iniciadas · {p1} reutilizaciones de cliente", { p0: count(stats.requests_started), p1: count(stats.clients_reused) })}</span>
      <span class="transport-toggle">{$t("Detalles")} <Icon name="chevron" size={16} /></span>
    </summary>
    <div class="transport-details">
      <p>
        {$t("Transporte compartido por páginas, robots y sitemaps. Los clientes del pool pueden reutilizarse; estos contadores no miden conexiones TCP. Los tiempos son acumulados y los cuerpos interrumpidos no equivalen a fallos de almacenamiento. Los timeouts de petición incluyen la espera por DNS o por un cliente disponible. Su desglose por conexión, cabeceras y plazo total no se suma de nuevo al total. Una consulta al resolver del sistema puede terminar después de cancelar la petición.")}
      </p>
      {#if readError || stale}
        <p class="transport-notice">{$t("Se muestra la última lectura disponible del transporte.")}</p>
      {/if}
      <div class="transport-groups">
        {#each groups as group}
          <section aria-label={$t(group.title)}>
            <h3>{$t(group.title)}</h3>
            <dl>
              {#each group.metrics as metric}
                <div><dt>{$t(metric.label)}</dt><dd>{count(metric.value)}{metric.value != null && metric.unit ? ` ${metric.unit}` : ""}</dd></div>
              {/each}
            </dl>
          </section>
        {/each}
      </div>
    </div>
  </details>
{/if}
