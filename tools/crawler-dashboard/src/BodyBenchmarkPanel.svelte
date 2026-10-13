<script lang="ts">
  import type { BodyBenchmarkReport, Distribution } from "./benchmark-types";

  let { report }: { report: BodyBenchmarkReport } = $props();
  let selectedIndex = $state(0);
  const selected = $derived(report.summaries[selectedIndex] ?? report.summaries[0]);
  const runs = $derived(report.runs.filter((run) => run.scenario === selected?.scenario));
  const number = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 2 });
  const display = (value: number | undefined, unit = "") => value == null ? "—" : `${number.format(value)}${unit ? ` ${unit}` : ""}`;
  const resources = [
    { key: "duration_ms", title: "Duración", unit: "ms" },
    { key: "server_body_bytes_written", title: "Bytes escritos por servidor", unit: "B" },
    { key: "body_bytes_received", title: "Bytes recibidos por cliente", unit: "B" },
    { key: "body_bytes_decoded", title: "Bytes decodificados", unit: "B" },
  ];
  const memory = [
    { key: "peak_reserved_bytes", title: "Reserva de cuerpos · pico", unit: "B" },
    { key: "rss_peak_bytes", title: "RSS · pico medido", unit: "B" },
    { key: "fd_peak", title: "Descriptores · pico medido", unit: "" },
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
  La aceptación comprueba las reglas y cotas de cada escenario; un rechazo esperado
  puede superar la verificación. Los bytes escritos por el servidor y los recibidos
  por el cliente son mediciones distintas: el transporte puede anticipar datos antes
  del corte. Este banco no mide persistencia WARC.
</p>
<div class="benchmark-scroll">
  <table class="benchmark-table" aria-label="Resultados de admisión y cuerpos">
    <thead><tr><th>Escenario</th><th>Resultado esperado</th><th>Repeticiones</th><th>Verificación</th>
      {#each resources as resource}<th>{resource.title}</th>{/each}
    </tr></thead>
    <tbody>{#each report.summaries as row}<tr>
      <th scope="row">{row.scenario}</th><td>{row.expected_outcome}</td><td>{row.repetitions}</td>
      <td class:benchmark-failed={!row.passed}>{row.passed ? "Reglas cumplidas" : "No superada"}</td>
      {#each resources as resource}<td class="benchmark-numeric">{@render metric(row.metrics[resource.key], resource.unit)}</td>{/each}
    </tr>{/each}</tbody>
  </table>
</div>
<div class="benchmark-details-grid">
  <div class="benchmark-detail">
    <div class="benchmark-stage-heading">
      <h3>Memoria y recursos</h3>
      <label><span class="sr-only">Escenario de cuerpos para inspeccionar</span><select bind:value={selectedIndex}>
        {#each report.summaries as row, index}<option value={index}>{row.scenario}</option>{/each}
      </select></label>
    </div>
    <div class="benchmark-scroll">
      <table class="benchmark-table" aria-label="Memoria del cuerpo por escenario">
        <thead><tr><th>Métrica</th><th>Mediana</th><th>MAD</th><th>Rango</th></tr></thead>
        <tbody>{#each memory as resource}
          {@const value = selected?.metrics[resource.key]}
          <tr><th scope="row">{resource.title}</th><td>{display(value?.median, resource.unit)}</td><td>{display(value?.mad, resource.unit)}</td><td>{value ? `${display(value.min, resource.unit)}–${display(value.max, resource.unit)}` : "—"}</td></tr>
        {/each}</tbody>
      </table>
    </div>
    <h3 class="benchmark-resource-title">Límites de esta medición</h3>
    <p>Los ajustes particulares de cada repetición se conservan en el informe descargado.</p>
    <details class="benchmark-provenance"><summary>Ver perfil y reglas verificadas</summary>
      <pre>{JSON.stringify({ profile: report.configuration.profile, checks: report.acceptance.checks }, null, 2)}</pre>
    </details>
  </div>
  <div class="benchmark-detail">
    <h3>Resultados por repetición</h3>
    <div class="benchmark-scroll">
      <table class="benchmark-table" aria-label="Resultados de cuerpos por repetición">
        <thead><tr><th>Repetición</th><th>Resultado</th><th>Recibidos por cliente</th><th>Decodificados</th><th>Escritos por servidor</th><th>Verificación</th></tr></thead>
        <tbody>{#each runs as run}<tr>
          <th scope="row">{run.repetition}</th>
          <td>{run.outcomes.map((outcome) => `${outcome.path}: ${outcome.cause}`).join(" · ")}</td>
          <td>{display(run.body_stats.wire_bytes_received, "B")}</td>
          <td>{display(run.body_stats.decoded_bytes_produced, "B")}</td>
          <td>{display(run.server.body_bytes_written, "B")}</td>
          <td class:benchmark-failed={!run.verification.passed}>{run.verification.passed ? "Reglas cumplidas" : "No superada"}</td>
        </tr>{:else}<tr><td colspan="6">Sin repeticiones publicadas.</td></tr>{/each}</tbody>
      </table>
    </div>
  </div>
</div>
<details class="benchmark-provenance">
  <summary>Código, binario, hardware y configuración de esta medición</summary>
  <pre>{JSON.stringify({ code: report.code, binary: report.binary, build: report.build, hardware: report.hardware, configuration: report.configuration, acceptance: report.acceptance }, null, 2)}</pre>
</details>
