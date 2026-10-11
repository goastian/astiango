<script lang="ts">
  import type { Snapshot } from "./types";

  let { counters }: { counters?: Snapshot["counters"] } = $props();
  const number = new Intl.NumberFormat("es-CO");
  const count = (value: number | undefined) =>
    value == null ? "—" : number.format(value);
  const requests = $derived([
    { label: "HTTP de páginas y sitemaps", value: counters?.page_http_attempts },
    { label: "Saltos de redirección", value: counters?.redirect_hops },
    { label: "Páginas útiles durables", value: counters?.useful_pages },
  ]);
  const outcomes = $derived([
    { label: "Omitidas", value: counters?.skipped },
    { label: "Fallos permanentes", value: counters?.failed },
    { label: "Aplazadas", value: counters?.deferred },
    { label: "Canceladas", value: counters?.cancelled },
    { label: "Fallos de almacenamiento", value: counters?.storage_failed },
  ]);
</script>

<section class="panel request-outcomes" aria-labelledby="requests-heading">
  <h2 id="requests-heading">Solicitudes y resultados</h2>
  <p class="card-description">
    Una página puede requerir varios intentos HTTP. El contador de HTTP incluye
    reintentos y sitemaps; excluye robots.txt. Sólo las páginas confirmadas por el
    almacenamiento cuentan como útiles durables.
  </p>
  <dl class="request-metrics">
    {#each requests as metric}
      <div><dt>{metric.label}</dt><dd>{count(metric.value)}</dd></div>
    {/each}
  </dl>
  <dl class="explicit-outcomes">
    {#each outcomes as outcome}
      <div><dt>{outcome.label}</dt><dd>{count(outcome.value)}</dd></div>
    {/each}
  </dl>
  <p class="card-footnote">
    Los aplazamientos y las cancelaciones conservan su resultado; no se cuentan
    como éxito. «—» indica que esta ejecución no publica ese contador.
  </p>
</section>
