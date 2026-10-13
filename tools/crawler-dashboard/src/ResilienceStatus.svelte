<script lang="ts">
  import type { Snapshot } from "./types";
  let { snapshot }: { snapshot?: Snapshot | null } = $props();
  const frontier = $derived(snapshot?.frontier_db);
  const format = new Intl.NumberFormat("es-CO");
  const number = (value?: number) => value === undefined ? "—" : format.format(value);
  let url = $state("");
  let message = $state("");
  let submitting = $state(false);
  async function release(event: SubmitEvent) {
    event.preventDefault(); submitting = true; message = "";
    try {
      const response = await fetch("/api/quarantine/release", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ url }) });
      const result = await response.json();
      if (!response.ok) throw new Error(result.error || "No se pudo guardar la revisión.");
      message = result.message;
      for (let attempt = 0; attempt < 10; attempt++) {
        await new Promise(resolve => setTimeout(resolve, 1000));
        const check = await fetch(`/api/control/${result.id}`);
        if (!check.ok) break;
        const completed = await check.json();
        if (completed.state === "queued") continue;
        message = completed.state === "completed" ? "URL liberada y admitida por el crawler." : completed.state === "not_found" ? "Esta URL no está en cuarentena." : "El crawler rechazó la revisión.";
        break;
      }
    } catch (error) { message = error instanceof Error ? error.message : "No se pudo guardar la revisión."; }
    finally { submitting = false; }
  }
</script>
{#if frontier && frontier.retry_deferred !== undefined}
  <details class="panel resilience-panel" open>
    <summary>Recuperación, descubrimiento y revalidación</summary>
    <div class="content">
      <div class="groups">
        <section aria-label="Reintentos C14"><h3>Reintentos durables</h3><dl>
          <div><dt>Aplazados / presupuesto agotado</dt><dd>{number(frontier.retry_deferred)} / {number(frontier.retry_exhausted)}</dd></div>
          <div><dt>Circuitos abiertos / recuperados</dt><dd>{number(frontier.circuits_opened)} / {number(frontier.retry_recovered)}</dd></div>
          <div><dt>Intentos HTTP persistidos</dt><dd>{number(frontier.http_attempts)}</dd></div>
        </dl></section>
        <section aria-label="Descubrimiento C15"><h3>Sitemaps y feeds</h3><dl>
          <div><dt>Candidatos distintos conservados</dt><dd>{number(frontier.discovery_candidates)}</dd></div>
          <div><dt>Documentos / bytes procesados</dt><dd>{number(frontier.discovery_documents)} / {number(frontier.discovery_bytes)} B</dd></div>
          <div><dt>Cortes por cuota / XML inválido</dt><dd>{number(frontier.discovery_quota_stops)} / {number(frontier.discovery_invalid_xml)}</dd></div>
        </dl></section>
        <section aria-label="Deduplicación C16"><h3>Versiones exactas</h3><dl>
          <div><dt>Payloads distintos / repetidos</dt><dd>{number(frontier.content_versions)} / {number(frontier.duplicate_content)}</dd></div>
          <div><dt>Páginas vacías</dt><dd>{number(frontier.empty_pages)}</dd></div>
        </dl><p>Un positivo Bloom se verifica contra claves exactas en disco.</p></section>
        <section aria-label="Trampas C17"><h3>Cuarentena reversible</h3><dl>
          <div><dt>URLs en cuarentena / liberadas</dt><dd>{number(Math.max(0, (frontier.quarantined_urls || 0) - (frontier.quarantine_released || 0)))} / {number(frontier.quarantine_released)}</dd></div>
          <div><dt>Muestras de revisión</dt><dd>{number(frontier.review_samples)}</dd></div>
          {#each Object.entries(frontier.quarantine_reasons || {}) as [reason, count]}<div><dt>{reason}</dt><dd>{number(count)}</dd></div>{/each}
        </dl></section>
        <section aria-label="Recrawl C18"><h3>Revalidación HTTP</h3><dl>
          <div><dt>304 con referencia durable</dt><dd>{number(frontier.revalidations)}</dd></div>
          <div><dt>Nuevas versiones por cambios</dt><dd>{number(frontier.recrawl_changes)}</dd></div>
          <div><dt>Payload evitado · bytes</dt><dd>{number(frontier.recrawl_bytes_saved)} B</dd></div>
        </dl><p>Las referencias conservan el payload anterior; un 304 no cuenta como página nueva.</p></section>
      </div>
      {#if frontier.quarantine_recent?.length}
        <p>Últimas exclusiones conservadas para revisión:</p>
        <ul class="recent">{#each frontier.quarantine_recent as item}<li><code>{item.url}</code><span>{item.reason}</span><button type="button" onclick={() => { url = item.url; message = "URL seleccionada para revisión."; }}>Seleccionar</button></li>{/each}</ul>
      {/if}
      <form onsubmit={release}>
        <label for="quarantine-url">URL en cuarentena que deseas liberar</label>
        <div class="controls"><input id="quarantine-url" type="url" bind:value={url} required placeholder="https://sitio.example/pagina" /><button type="submit" disabled={submitting}>{submitting ? "Guardando…" : "Liberar URL"}</button></div>
        {#if message}<p role="status">{message}</p>{/if}
      </form>
    </div>
  </details>
{/if}
<style>
  .recent { list-style: none; padding: 0; } .recent li { display: flex; align-items: center; gap: 12px; border-bottom: 1px solid #eef0f4; padding: 8px 0; font-size: 12px; } .recent code { overflow-wrap: anywhere; flex: 1; }
  .resilience-panel { margin-top: 20px; }
  summary { padding: 20px 24px; cursor: pointer; font-size: 12px; font-weight: 600; }
  .content { padding: 0 24px 20px; }
  .groups { display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: 24px; }
  h3 { font-size: 12px; } p, label { font-size: 12px; color: #697386; line-height: 1.6; }
  dl div { display: flex; justify-content: space-between; gap: 16px; padding: 8px 0; border-bottom: 1px solid #eef0f4; font-size: 12px; }
  dd { margin: 0; text-align: right; } form { margin-top: 20px; max-width: 750px; }
  .controls { display: flex; gap: 12px; margin-top: 8px; } input { flex: 1; min-width: 0; padding: 10px; border: 1px solid #ccd4e2; border-radius: 4px; }
  button { padding: 10px 16px; border: 1px solid #ccd4e2; background: #fff; border-radius: 4px; cursor: pointer; }
  @media (max-width: 900px) { .groups { grid-template-columns: 1fr; } .controls { flex-direction: column; } }
</style>
