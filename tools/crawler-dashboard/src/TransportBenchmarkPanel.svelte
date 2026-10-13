<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, locale, formatLocale } = getI18n();

  import type { Distribution, TransportBenchmarkReport } from "./benchmark-types";

  let { report }: { report: TransportBenchmarkReport } = $props();
  let selectedIndex = $state(0);
  const selected = $derived(report.summaries[selectedIndex] ?? report.summaries[0]);
  const runs = $derived(report.runs.filter((run) => selected && run.scheme === selected.scheme &&
    run.origins === selected.origins && run.mode === selected.mode));
  const number = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 4 }));
  const display = (value: number | undefined, unit = "") => value == null ? "—" : `${number.format(value)}${unit ? ` ${unit}` : ""}`;
  const mode = (value: string) => value === "pooled" ? "Cliente compartido" : "Cliente por petición";
  const resources = [
    { key: "duration_ms", title: "Duración", unit: "ms" },
    { key: "pages", title: "Páginas leídas", unit: "" },
    { key: "connections", title: "Conexiones TCP", unit: "" },
    { key: "connections_per_page", title: "Conexiones por página", unit: "" },
    { key: "dns_queries_per_page", title: "Consultas DNS por página", unit: "" },
  ];
  const details = [
    { key: "requests", title: "Peticiones", unit: "" },
    { key: "dns_queries", title: "Consultas DNS", unit: "" },
    { key: "fd_baseline", title: "Descriptores iniciales", unit: "" },
    { key: "fd_peak", title: "Descriptores · pico medido", unit: "" },
    { key: "fd_final", title: "Descriptores finales", unit: "" },
  ];
</script>

{#snippet metric(value: Distribution | undefined, unit = "")}
  <strong>{display(value?.median, unit)}</strong>
  {#if value}<small>{$t("MAD {p0}", { p0: display(value.mad, unit) })}</small><small>{display(value.min, unit)}–{display(value.max, unit)}</small>{/if}
{/snippet}

<div class="benchmark-summary">
  <span class="benchmark-result" class:passed={report.acceptance.passed}>{report.acceptance.passed ? $t("Aceptación verificada") : $t("Aceptación pendiente")}</span>
  <span>{$t("{p0} escenarios · {p1} (Bogotá)", { p0: report.summaries.length, p1: new Date(report.generated_at).toLocaleString($formatLocale, { timeZone: "America/Bogota" }) })}</span>
  <span>{$t("Mediana · MAD · rango mínimo–máximo")}</span>
</div>
<p class="benchmark-acceptance-note">
  {$t("Las conexiones TCP son sockets aceptados por el servidor de prueba. Las páginas corresponden a cuerpos leídos y verificados; este banco no mide persistencia WARC. Sus resultados están separados del rastreo operativo.")}
</p>
<div class="benchmark-scroll">
  <table class="benchmark-table" aria-label={$t("Resultados del transporte")}>
    <thead><tr><th>{$t("Protocolo")}</th><th>{$t("Orígenes")}</th><th>{$t("Transporte")}</th><th>{$t("Repeticiones")}</th>
      {#each resources as resource}<th>{$t(resource.title)}</th>{/each}
    </tr></thead>
    <tbody>{#each report.summaries as row}<tr>
      <th scope="row">{row.scheme.toUpperCase()}</th><td>{row.origins}</td><td>{$t(mode(row.mode))}</td><td>{row.repetitions}</td>
      {#each resources as resource}<td class="benchmark-numeric">{@render metric(row.metrics[resource.key], resource.unit)}</td>{/each}
    </tr>{/each}</tbody>
  </table>
</div>
<div class="benchmark-details-grid">
  <div class="benchmark-detail">
    <h3>{$t("Comparación del transporte")}</h3>
    <p>{$t("Reducción medida frente al cliente por petición. Un porcentaje positivo indica menos recursos por página; se conservan las diferencias de carga.")}</p>
    <div class="benchmark-scroll">
      <table class="benchmark-table" aria-label={$t("Comparación del transporte")}>
        <thead><tr><th>{$t("Protocolo")}</th><th>{$t("Orígenes")}</th><th>{$t("Reducción de conexiones por página")}</th><th>{$t("Reducción de DNS por página")}</th><th>{$t("Misma carga")}</th></tr></thead>
        <tbody>{#each report.comparisons ?? [] as row}<tr>
          <th scope="row">{row.scheme.toUpperCase()}</th><td>{row.origins}</td>
          <td class="benchmark-numeric">{@render metric(row.connections_per_page_reduction_pct, "%")}</td>
          <td class="benchmark-numeric">{@render metric(row.dns_queries_per_page_reduction_pct, "%")}</td>
          <td class:benchmark-failed={!row.equivalent}>{row.equivalent ? $t("Carga equivalente") : $t("Diferencias de carga")}</td>
        </tr>{:else}<tr><td colspan="5">{$t("Sin comparación publicada.")}</td></tr>{/each}</tbody>
      </table>
    </div>
  </div>
  <div class="benchmark-detail">
    <div class="benchmark-stage-heading">
      <h3>{$t("Recursos y repeticiones")}</h3>
      <label><span class="sr-only">{$t("Escenario de transporte para inspeccionar")}</span><select bind:value={selectedIndex}>
        {#each report.summaries as row, index}<option value={index}>{$t("{p0} · {p1} orígenes · {p2}", { p0: row.scheme.toUpperCase(), p1: row.origins, p2: mode(row.mode) })}</option>{/each}
      </select></label>
    </div>
    <div class="benchmark-scroll">
      <table class="benchmark-table" aria-label={$t("Recursos del transporte por escenario")}>
        <thead><tr><th>{$t("Métrica")}</th><th>{$t("Mediana")}</th><th>{$t("MAD")}</th><th>{$t("Rango")}</th></tr></thead>
        <tbody>{#each details as resource}
          {@const value = selected?.metrics[resource.key]}
          <tr><th scope="row">{$t(resource.title)}</th><td>{display(value?.median, resource.unit)}</td><td>{display(value?.mad, resource.unit)}</td><td>{value ? `${display(value.min, resource.unit)}–${display(value.max, resource.unit)}` : "—"}</td></tr>
        {/each}</tbody>
      </table>
    </div>
    <h3 class="benchmark-resource-title">{$t("Conexiones medidas por repetición")}</h3>
    <div class="benchmark-scroll">
      <table class="benchmark-table" aria-label={$t("Conexiones reales por repetición")}>
        <thead><tr><th>{$t("Repetición")}</th><th>{$t("Páginas")}</th><th>{$t("Peticiones cliente / servidor")}</th><th>{$t("Conexiones TCP")}</th><th>{$t("Verificación")}</th></tr></thead>
        <tbody>{#each runs as run}<tr>
          <th scope="row">{run.repetition}</th><td>{display(run.pages)}</td><td>{display(run.requests)} / {display(run.server.requests)}</td>
          <td>{display(run.server.connections)}</td><td class:benchmark-failed={run.verification?.passed === false}>{run.verification?.passed === true ? $t("Verificada") : run.verification?.passed === false ? $t("No superada") : $t("No publicada")}</td>
        </tr>{:else}<tr><td colspan="5">{$t("Sin repeticiones publicadas.")}</td></tr>{/each}</tbody>
      </table>
    </div>
  </div>
</div>
<details class="benchmark-provenance">
  <summary>{$t("Código, corpus, hardware y configuración de esta medición")}</summary>
  <pre>{JSON.stringify({ code: report.code, corpus: report.corpus, hardware: report.hardware, configuration: report.configuration, source_unchanged: report.source_unchanged, acceptance: report.acceptance }, null, 2)}</pre>
</details>
