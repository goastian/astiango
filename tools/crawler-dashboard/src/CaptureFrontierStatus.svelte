<script lang="ts">
  import Icon from "./Icon.svelte";
  import type { FrontierDbStats, SpoolStats } from "./types";
  let { frontier, spool }: { frontier?: FrontierDbStats; spool?: SpoolStats } = $props();
  const number = new Intl.NumberFormat("es-CO", { maximumFractionDigits: 2 });
  const display = (value: number) => number.format(value);
  const mib = (value: number) => display(value / 1024 / 1024) + " MiB";
  const rows = $derived(frontier ? [
    ["URLs conservadas", display(frontier.urls)], ["URLs pendientes", display(frontier.pending_urls)],
    ["URLs con reserva activa", display(frontier.leased_urls)], ["URLs archivadas", display(frontier.committed_urls)],
    ["URLs resueltas con causa", display(frontier.terminal_urls)], ["Capturas confirmadas", display(frontier.commits)],
    ["Descubrimientos pendientes de publicar", display(frontier.outbox_entries)],
    ["Reservas recuperadas", display(frontier.recovered_leases)], ["Capturas reconciliadas desde WARC", display(frontier.reconciled_captures)],
    ["Caché fija", mib(frontier.cache_bytes)], ["Máximo de URLs por lote", display(frontier.batch_size)],
    ["Jobs procesados en migración", display(frontier.migrated_jobs)],
    ["Migración", frontier.migration_complete ? "Completada" : "En curso"],
  ] : []);
</script>
{#if frontier || spool?.capture_records !== undefined}
  <details class="panel capture-frontier-panel">
    <summary><strong>Captura HTTP y frontera transaccional</strong>
      <span class="summary">{frontier ? display(frontier.urls) + " URLs conservadas" : "Captura WARC 1.1"}</span>
      <span class="toggle">Detalles <Icon name="chevron" size={16} /></span>
    </summary>
    <div class="content"><p>La captura conserva el cuerpo recibido y su SHA-256 antes de decodificarlo. Las decisiones de indexación acompañan al archivo.
      Cada reserva, archivo y commit comparte un identificador; la recuperación también publica los enlaces pendientes.</p>
      <div class="groups">
        {#if spool?.capture_records !== undefined}<section aria-label="Captura C09"><h3>Capturas de esta ejecución</h3><dl>
          <div><dt>Capturas WARC 1.1 durables</dt><dd>{display(spool.capture_records)}</dd></div>
          <div><dt>Representación recibida</dt><dd>{mib(spool.captured_payload_bytes ?? 0)}</dd></div>
          <div><dt>Vista decodificada</dt><dd>{mib(spool.decoded_payload_bytes ?? 0)}</dd></div>
          <div><dt>Archivadas con noindex</dt><dd>{display(spool.noindex_records ?? 0)}</dd></div>
          <div><dt>Archivadas con nofollow</dt><dd>{display(spool.nofollow_records ?? 0)}</dd></div>
        </dl><p>Noindex permite archivar la respuesta y excluye su contenido del índice. Nofollow excluye sus enlaces del descubrimiento y del webgraph.</p></section>{/if}
        {#if frontier}<section aria-label="Frontera C10"><h3>Frontera durable · redb</h3><dl>
          {#each rows as [label, value]}<div><dt>{label}</dt><dd>{value}</dd></div>{/each}
        </dl><p>Estos totales pertenecen al estado conservado entre reinicios. Los jobs procesados en migración incluyen sus relecturas tras una caída; las URLs se conservan una sola vez. Los resultados del banco local aparecen en su propia pestaña.</p></section>{/if}
      </div>
    </div>
  </details>
{/if}
<style>
  .capture-frontier-panel { margin-top: 20px; }
  summary { display: flex; align-items: center; gap: 16px; padding: 20px 24px; cursor: pointer; list-style: none; }
  summary::-webkit-details-marker { display: none; }
  summary strong, h3 { font-size: 12px; }
  .summary, .toggle { font-size: 11px; color: #7d8799; }
  .toggle { margin-left: auto; display: flex; align-items: center; gap: 8px; }
  .content { padding: 0 24px 20px; }
  .groups { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 28px; }
  p { font-size: 12px; color: #697386; line-height: 1.6; }
  dl div { display: flex; justify-content: space-between; gap: 20px; padding: 8px 0; border-bottom: 1px solid #eef0f4; font-size: 12px; }
  dd { margin: 0; text-align: right; }
  @media (max-width: 700px) { summary { flex-wrap: wrap; gap: 10px; } .groups { grid-template-columns: 1fr; } .content { padding: 0 16px 16px; } }
</style>
