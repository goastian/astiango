<script lang="ts">
  import type { Distribution, TransportBenchmarkReport } from "./benchmark-types";

  let { report }: { report: TransportBenchmarkReport } = $props();
  let selectedIndex = $state(0);
  const selected = $derived(report.summaries[selectedIndex] ?? report.summaries[0]);
  const runs = $derived(report.runs.filter((run) => selected && run.scheme === selected.scheme &&
    run.origins === selected.origins && run.mode === selected.mode));
  const number = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 4 });
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
  {#if value}<small>MAD {display(value.mad, unit)}</small><small>{display(value.min, unit)}–{display(value.max, unit)}</small>{/if}
{/snippet}

<div class="benchmark-summary">
  <span class="benchmark-result" class:passed={report.acceptance.passed}>{report.acceptance.passed ? "Aceptación verificada" : "Aceptación pendiente"}</span>
  <span>{report.summaries.length} escenarios · {new Date(report.generated_at).toLocaleString("es-CO", { timeZone: "America/Bogota" })} (Bogotá)</span>
  <span>Mediana · MAD · rango mínimo–máximo</span>
</div>
<p class="benchmark-acceptance-note">
  Las conexiones TCP son sockets aceptados por el servidor de prueba. Las páginas
  corresponden a cuerpos leídos y verificados; este banco no mide persistencia WARC.
  Sus resultados están separados del rastreo operativo.
</p>
<div class="benchmark-scroll">
  <table class="benchmark-table" aria-label="Resultados del transporte C04">
    <thead><tr><th>Protocolo</th><th>Orígenes</th><th>Transporte</th><th>Repeticiones</th>
      {#each resources as resource}<th>{resource.title}</th>{/each}
    </tr></thead>
    <tbody>{#each report.summaries as row}<tr>
      <th scope="row">{row.scheme.toUpperCase()}</th><td>{row.origins}</td><td>{mode(row.mode)}</td><td>{row.repetitions}</td>
      {#each resources as resource}<td class="benchmark-numeric">{@render metric(row.metrics[resource.key], resource.unit)}</td>{/each}
    </tr>{/each}</tbody>
  </table>
</div>
<div class="benchmark-details-grid">
  <div class="benchmark-detail">
    <h3>Comparación del transporte</h3>
    <p>Reducción medida frente al cliente por petición. Un porcentaje positivo indica menos recursos por página; se conservan las diferencias de carga.</p>
    <div class="benchmark-scroll">
      <table class="benchmark-table" aria-label="Comparación del transporte C04">
        <thead><tr><th>Protocolo</th><th>Orígenes</th><th>Reducción de conexiones por página</th><th>Reducción de DNS por página</th><th>Misma carga</th></tr></thead>
        <tbody>{#each report.comparisons ?? [] as row}<tr>
          <th scope="row">{row.scheme.toUpperCase()}</th><td>{row.origins}</td>
          <td class="benchmark-numeric">{@render metric(row.connections_per_page_reduction_pct, "%")}</td>
          <td class="benchmark-numeric">{@render metric(row.dns_queries_per_page_reduction_pct, "%")}</td>
          <td class:benchmark-failed={!row.equivalent}>{row.equivalent ? "Carga equivalente" : "Diferencias de carga"}</td>
        </tr>{:else}<tr><td colspan="5">Sin comparación publicada.</td></tr>{/each}</tbody>
      </table>
    </div>
  </div>
  <div class="benchmark-detail">
    <div class="benchmark-stage-heading">
      <h3>Recursos y repeticiones</h3>
      <label><span class="sr-only">Escenario de transporte para inspeccionar</span><select bind:value={selectedIndex}>
        {#each report.summaries as row, index}<option value={index}>{row.scheme.toUpperCase()} · {row.origins} orígenes · {mode(row.mode)}</option>{/each}
      </select></label>
    </div>
    <div class="benchmark-scroll">
      <table class="benchmark-table" aria-label="Recursos del transporte por escenario">
        <thead><tr><th>Métrica</th><th>Mediana</th><th>MAD</th><th>Rango</th></tr></thead>
        <tbody>{#each details as resource}
          {@const value = selected?.metrics[resource.key]}
          <tr><th scope="row">{resource.title}</th><td>{display(value?.median, resource.unit)}</td><td>{display(value?.mad, resource.unit)}</td><td>{value ? `${display(value.min, resource.unit)}–${display(value.max, resource.unit)}` : "—"}</td></tr>
        {/each}</tbody>
      </table>
    </div>
    <h3 class="benchmark-resource-title">Conexiones medidas por repetición</h3>
    <div class="benchmark-scroll">
      <table class="benchmark-table" aria-label="Conexiones reales por repetición">
        <thead><tr><th>Repetición</th><th>Páginas</th><th>Peticiones cliente / servidor</th><th>Conexiones TCP</th><th>Verificación</th></tr></thead>
        <tbody>{#each runs as run}<tr>
          <th scope="row">{run.repetition}</th><td>{display(run.pages)}</td><td>{display(run.requests)} / {display(run.server.requests)}</td>
          <td>{display(run.server.connections)}</td><td class:benchmark-failed={run.verification?.passed === false}>{run.verification?.passed === true ? "Verificada" : run.verification?.passed === false ? "No superada" : "No publicada"}</td>
        </tr>{:else}<tr><td colspan="5">Sin repeticiones publicadas.</td></tr>{/each}</tbody>
      </table>
    </div>
  </div>
</div>
<details class="benchmark-provenance">
  <summary>Código, corpus, hardware y configuración de esta medición</summary>
  <pre>{JSON.stringify({ code: report.code, corpus: report.corpus, hardware: report.hardware, configuration: report.configuration, source_unchanged: report.source_unchanged, acceptance: report.acceptance }, null, 2)}</pre>
</details>
