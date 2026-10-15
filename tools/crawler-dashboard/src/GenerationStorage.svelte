<script lang="ts">
  import { getI18n } from "./i18n";
  import type { GenerationStorage } from "./generation-storage";
  import type { IndexingResponse, WebgraphResponse } from "./types";
  const { t, formatLocale } = getI18n();
  let { storage = null, indexing = null, webgraph = null, component = "all", now }: {
    storage?: GenerationStorage | null; indexing?: IndexingResponse | null;
    webgraph?: WebgraphResponse | null; component?: string; now: number;
  } = $props();
  const fresh = $derived(storage && now - storage.updated_at * 1000 < 60_000);
  const rows = $derived([
    { id: "indexer", name: "Indexing", value: indexing?.snapshot.totals.output_bytes,
      quota: indexing?.snapshot.configuration.max_output_bytes, blocked: indexing?.snapshot.phase === "blocked" },
    { id: "webgraph", name: "Webgraph", value: webgraph?.snapshot.totals.output_bytes,
      quota: webgraph?.snapshot.configuration.max_output_bytes, blocked: webgraph?.snapshot.phase === "blocked" },
  ].filter(row => component === "all" || component === row.id));
  const format = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 2 }));
  const bytes = (value: number) => `${format.format(value / 1073741824)} GiB`;
</script>

<section class="generation-storage panel" aria-label={$t("Generation storage")}>
  <header><h2>{$t("Generation storage")}</h2><span>{$t("Active and rollback versions protected")}</span></header>
  <div class="storage-rows">
    {#each rows as row}
      {@const used = row.value ?? storage?.components[row.id]?.working_bytes}
      {@const quota = row.quota ?? storage?.components[row.id]?.working_quota_bytes}
      {@const history = fresh ? storage?.components[row.id] : undefined}
      {#if used !== undefined && quota}
        {@const percentage = 100 * used / quota}
        <article class:pressure={row.blocked || percentage >= 90}>
          <div class="working"><strong>{$t(row.name)}</strong><span>{$t("Working storage")}</span>
            <b>{bytes(used)} <small>/ {bytes(quota)}</small></b>
            <div class="capacity-track" role="meter" aria-label={`${$t(row.name)} · ${$t("Working storage")}`} aria-valuenow={Math.min(100,Math.max(0,Math.round(percentage)))} aria-valuemin="0" aria-valuemax="100"><i style:width={`${Math.min(100,percentage)}%`}></i></div>
            <em>{format.format(percentage)}%</em>
          </div>
          <div class="cold"><span>{$t("Recoverable history")}</span><b>{history ? history.archived_generations : "—"} <small>{$t("Archived generations")}</small></b><span>{history ? bytes(history.archive_bytes) : "—"}</span></div>
        </article>
      {/if}
    {/each}
  </div>
  {#if storage?.error}<p class="storage-error" role="status">{$t("History maintenance needs attention")}: {storage.error}</p>
  {:else if storage && !fresh}<p class="storage-error" role="status">{$t("History status is stale")}</p>{/if}
</section>

<style>
  .generation-storage{padding:22px 24px;margin:0 0 22px}header{display:flex;justify-content:space-between;gap:12px;flex-wrap:wrap;margin-bottom:20px}h2{font-size:15px;margin:0}header>span,.working>span,.cold>span{font-size:11px;color:#68756d}.storage-rows{display:grid;gap:18px}article{display:grid;grid-template-columns:minmax(0,2fr) minmax(0,1fr);gap:24px}.working{display:grid;grid-template-columns:90px 1fr auto;gap:8px 14px;align-items:center}.working strong{font-size:12px;grid-row:span 2}.working b{font-size:14px;font-variant-numeric:tabular-nums;font-weight:600}small{font-size:11px;font-weight:400;color:#68756d}.capacity-track{height:6px;background:#edf1ed;overflow:hidden}.capacity-track i{height:100%;display:block;background:#377960}.working em{font-style:normal;font-size:11px;text-align:right;color:#68756d}.cold{border-left:1px solid #dfe6df;padding-left:24px;display:grid;grid-template-columns:1fr auto;gap:6px 12px}.cold>b{grid-column:1;font-size:15px;font-variant-numeric:tabular-nums}.cold>span:last-child{grid-column:2;grid-row:2;text-align:right}.pressure .capacity-track i{background:#b07720}.pressure .working b{color:#95651c}.storage-error{font-size:12px;color:#8a5c17;margin:18px 0 0;overflow-wrap:anywhere}@media(max-width:900px){article{grid-template-columns:1fr}.cold{border-left:0;padding:0}.working{grid-template-columns:75px 1fr auto}}@media(max-width:540px){.generation-storage{padding:18px}.working{grid-template-columns:1fr auto}.working strong{grid-column:1;grid-row:auto}.working>span{display:none}.working>b{grid-column:2}.cold small{display:block}}
</style>
