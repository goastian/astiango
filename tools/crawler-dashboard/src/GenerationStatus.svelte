<script lang="ts">
  import { getI18n } from "./i18n";
  import { onMount } from "svelte";
  const { t, formatLocale } = getI18n();
  let { now, paused = false }: { now: number; paused?: boolean } = $props();
  type Generation = { id: string; state: string; documents: number; created_at: string; logical_sha256: string; elapsed_ms: number; rebuilt_buckets: number; reused_buckets: number; error?: string };
  let data = $state<{ version: number; active: string | null; previous: string | null; generations: Generation[]; artifact?: { build_id: string; engine: string }; last_error?: {error:string}|null; updated_at: string } | null>(null);
  let error = $state("");
  async function refresh() {
    if (paused) return;
    try {
      const response = await fetch("/api/indexing/generations");
      if (response.status === 404) { error = "No validated generation yet"; return; }
      if (!response.ok) throw new Error();
      data = await response.json(); error = "";
    } catch { error = "Generation status unavailable"; }
  }
  onMount(() => { refresh(); const timer = setInterval(refresh, 3000); return () => clearInterval(timer); });
  const active = $derived(data?.generations.find(row => row.id === data?.active));
  const latest = $derived(data?.generations[0]);
  const number = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 1 }));
  const date = $derived(new Intl.DateTimeFormat($formatLocale, { month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" }));
  const states: Record<string,string> = { ready:"Ready", preparing:"Preparing", failed:"Failed", abandoned:"Interrupted", active:"Active generation" };
</script>

<div class="generation-view">
  <section class="release-line panel">
    {#each ["Ingestion", "Engine committed", "Content validated", "Active generation"] as stage, i}
      <div class:passed={!!data && (i < 2 || (i === 2 && latest?.state === "ready") || (i === 3 && !!active))}><span>{String(i+1).padStart(2,"0")}</span><h2>{$t(stage)}</h2><i></i></div>
    {/each}
  </section>
  {#if error}<p class="notice" role="status">{$t(error)}</p>{/if}
  {#if data?.last_error}<p class="notice" role="status">{data.last_error.error}</p>{/if}
  <section class="release-summary">
    <article class="panel active"><span>{$t("Active generation")}</span><strong>{active ? number.format(active.documents) : "—"}</strong><span>{$t("Searchable documents")}</span><code>{data?.active?.slice(0,16) ?? "—"}</code><small>{$t("Local serving selector")}</small></article>
    <article class="panel"><h2>{$t("Incremental publication")}</h2>
      <div class="bucket-bar" aria-label={$t("Reused and rebuilt partitions")}>{#each Array.from({length:16},(_,i)=>i) as i}<i class:reused={i < (latest?.reused_buckets ?? 0)}></i>{/each}</div>
      <dl><div><dt>{$t("Reused")}</dt><dd>{latest?.reused_buckets ?? "—"} / 16</dd></div><div><dt>{$t("Rebuilt")}</dt><dd>{latest?.rebuilt_buckets ?? "—"} / 16</dd></div><div><dt>{$t("Preparation time")}</dt><dd>{latest ? number.format(latest.elapsed_ms/1000)+" s" : "—"}</dd></div></dl>
    </article>
    <article class="panel rollback"><h2>{$t("Retained rollback")}</h2><code>{data?.previous?.slice(0,16) ?? "—"}</code><span>{$t(data?.previous ? "Previous generation retained" : "No previous generation")}</span><small>{$t("Capture garbage collection remains disabled")}</small></article>
  </section>
  <section class="panel catalog"><header><h2>{$t("Generation catalog")}</h2><small>{$t("Validated before selection")}</small></header>
    <div class="table-scroll"><table><thead><tr><th>{$t("Generation")}</th><th>{$t("State")}</th><th>{$t("Documents")}</th><th>{$t("Created")}</th><th>{$t("Content fingerprint")}</th></tr></thead><tbody>
      {#each data?.generations ?? [] as row}<tr><td><code>{row.id.slice(0,16)}</code></td><td><span class:ready={row.state === "ready"}>{$t(row.id === data?.active ? "Active generation" : states[row.state] ?? row.state)}</span>{#if row.error}<small>{row.error}</small>{/if}</td><td>{number.format(row.documents)}</td><td>{date.format(new Date(row.created_at))}</td><td><code>{row.logical_sha256.slice(0,16)}</code></td></tr>{/each}
    </tbody></table></div>
  </section>
  <footer><span>{$t("Build identity")} <code>{data?.artifact?.build_id?.slice(0,16) ?? "—"}</code></span><span>{$t("Last sample")} {data ? date.format(new Date(data.updated_at)) : "—"}</span><span>{$t("Public load rollout remains a serving gate")}</span></footer>
</div>

<style>
 .generation-view {display:grid;gap:20px;min-width:0;font-variant-numeric:tabular-nums;} .panel {padding:24px;min-width:0;}
 .release-line {display:grid;grid-template-columns:repeat(4,1fr);gap:24px;background:#f5f8f5;} .release-line span {font:12px monospace;color:#748177;} h2 {margin:0;font-size:15px;} .release-line h2 {margin:8px 0 14px;} .release-line i {display:block;height:3px;background:#dce3dd;} .release-line .passed i {background:#367b62;}
 .release-summary {display:grid;grid-template-columns:1fr 2fr 1fr;gap:16px;} .active {border-left:3px solid #367b62;} .active strong {display:block;font-size:42px;letter-spacing:-1.5px;margin:10px 0 3px;} .active span,.rollback span,small {font-size:12px;color:#637368;display:block;} code {display:block;font-size:12px;overflow-wrap:anywhere;margin-top:12px;color:#384b3d;} small {margin-top:12px;} .rollback code {font-size:17px;margin:24px 0 14px;}
 .bucket-bar {display:flex;gap:4px;height:42px;margin:28px 0 18px;} .bucket-bar i {flex:1;background:#c9d4cc;} .bucket-bar .reused {background:#367b62;} dl {display:flex;gap:28px;margin:0;flex-wrap:wrap;} dt {font-size:12px;color:#637368;} dd {margin:5px 0 0;font-size:18px;} .catalog header {display:flex;justify-content:space-between;gap:12px;flex-wrap:wrap;} .catalog header small {margin:0;} .table-scroll {overflow-x:auto;} table {width:100%;border-collapse:collapse;margin-top:20px;font-size:12px;text-align:left;} th,td {border-bottom:1px solid #e2e8e3;padding:14px 10px;vertical-align:top;} td code {margin:0;} .ready {color:#256a55;} footer {display:flex;gap:20px;flex-wrap:wrap;font-size:12px;color:#637368;} footer code {display:inline;margin:0 0 0 6px;}
 @media(max-width:1000px) {.release-summary {grid-template-columns:1fr 1fr;} .release-summary article:nth-child(2) {grid-column:1/-1;grid-row:2;}}
 @media(max-width:540px) {.release-line {grid-template-columns:1fr 1fr;gap:18px;} .panel {padding:18px;} .release-summary {gap:10px;} .active strong {font-size:32px;}}
</style>
