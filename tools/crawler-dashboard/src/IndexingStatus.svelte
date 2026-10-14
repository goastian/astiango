<script lang="ts">
  import { onMount } from "svelte";
  import { getI18n } from "./i18n";
  import WorkPulse from "./WorkPulse.svelte";
  import SignalsStatus from "./SignalsStatus.svelte";
  import type { ComponentFlow } from "./component-flow";
  import TimeChart from "./TimeChart.svelte";
  import Pagination from "./Pagination.svelte";
  import type { IndexingResponse, Snapshot } from "./types";
  import { consumerActivity } from "./work-status";
  const { t, formatLocale } = getI18n();
  let { flow = null, response, crawler, now, readError = "", paused = false, mode = "overview" }: {
    flow?:ComponentFlow|null; response: IndexingResponse | null; crawler?: Snapshot; now: number; readError?: string; paused?: boolean; mode?: string;
  } = $props();
  let generation = $state<{active:string|null}|null>(null);
  onMount(() => {
    const read = async () => { if (paused) return; try { const response = await fetch("/api/indexing/generations"); if (response.ok) generation = await response.json(); } catch {} };
    read(); const timer = setInterval(read,3000); return () => clearInterval(timer);
  });
  let page = $state(1);
  let filter = $state("all");
  let query = $state("");
  let previewQuery = $state("");
  let preview = $state<Array<{url:string; title:string; snippet:string}>>([]);
  let previewState = $state("");
  async function searchPreview() {
    previewState = "Loading";
    try {
      const response = await fetch(`/api/indexing/preview?q=${encodeURIComponent(previewQuery)}`);
      if (!response.ok) throw new Error("Preview unavailable; retry between ingestion jobs");
      preview = (await response.json()).results;
      previewState = preview.length ? "" : "No results";
    } catch { previewState = "Preview unavailable; retry between ingestion jobs"; }
  }
  const snapshot = $derived(response?.snapshot);
  const totals = $derived(snapshot?.totals);
  const activity = $derived(consumerActivity(snapshot,crawler));
  const format = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 1 }));
  const date = $derived(new Intl.DateTimeFormat($formatLocale, { hour: "2-digit", minute: "2-digit", second: "2-digit" }));
  const number = (value?: number | null) => value == null ? "—" : format.format(value);
  const bytes = (value?: number) => value === undefined ? "—" : `${number(value / 1048576)} MiB`;
  const states: Record<string, string> = { indexing: "Indexing", idle: "Waiting", blocked: "Backpressure", draining: "Draining", stopped: "Stopped", pending: "Pending", running: "Indexing", completed: "Committed", failed: "Failed", deferred:"Deferred" };
  const reasons: Record<string, string> = {
    processing_segment: "Processing sealed segment", waiting_for_segments: "Waiting for sealed segments", admission_interval: "Admission interval",
    pilot_limit: "Pilot shard limit reached", output_quota: "Index output quota reached", disk_reserve: "Disk reserve reached",
    index_pressure:"Index resource pressure",
    crawler_pressure: "Crawler resource pressure", crawler_unavailable: "Crawler state unavailable", source_unavailable: "Input spool unavailable", shutdown: "Stopped",
  };
  const stale = $derived(!!snapshot && now - Date.parse(snapshot.updated_at) > 10_000);
  const unavailable = $derived(!!readError || stale || response?.meta.process_alive === false);
  const filtered = $derived((snapshot?.shards ?? []).filter(row => (filter === "all" || row.state === filter) && `${row.file} ${row.error ?? ""}`.toLowerCase().includes(query.toLowerCase())));
  const rows = $derived(filtered.slice((page - 1) * 10, page * 10));
  $effect(() => { filter; query; page = 1; });
  const rate = $derived(totals && totals.elapsed_seconds > 0 ? totals.indexed_documents / totals.elapsed_seconds : undefined);
  const cpuPerDocument = $derived(totals && totals.indexed_documents > 0 ? totals.cpu_seconds * 1000 / totals.indexed_documents : undefined);
  const history = $derived(snapshot?.history ?? []);
  const throughput = $derived(history.map((sample, i) => ({ at: sample.at, value: i ? Math.max(0, sample.documents-history[i-1].documents)/Math.max(.001,(Date.parse(sample.at)-Date.parse(history[i-1].at))/60000) : 0 })));
  const rss = $derived(history.map(sample => ({ at: sample.at, value: sample.rss_mib })));
  const cpu = $derived(history.map(sample => ({ at: sample.at, value: sample.cpu_percent })));
  const stageNames: Record<string,string> = {prepare:"Indexing",committed:"Committed",failed:"Failed",read_decode:"Read and decode",dom:"HTML parsing",text_extraction:"Text extraction",signals:"Ranking signals",tokenization:"Tokenization",commit:"Commit",merge:"Merge"};
  const outcomeNames: Record<string,string> = {indexed:"Indexed",noindex:"Noindex",duplicate:"Duplicate",excluded:"Excluded",revalidated:"Revalidated",pending:"Pending",deferred:"Deferred"};
  const stages = $derived(Object.entries(snapshot?.runtime?.stages ?? {}));
  const stageMax = $derived(Math.max(1,...stages.map(([,value])=>value.cpu_ms)));
  const outcomes = $derived(Object.entries(snapshot?.runtime?.outcomes ?? {}));
  const outcomeMax = $derived(Math.max(1,...outcomes.map(([,value])=>value)));
</script>

<div class="indexing-content">
  {#if mode === "overview"}<WorkPulse {flow} component="indexer" /><SignalsStatus {flow} compact />{/if}
  <section class="panel indexing-health" aria-label={$t("Indexing service")}>
    <div><h2>{$t("Indexing service")}</h2><p>{snapshot ? $t(activity.detail) : $t("Indexing state is unavailable.")}</p></div>
    <span class:warning={unavailable || activity.warning}>{unavailable ? $t("State unavailable or stale") : snapshot ? $t(activity.label) : "—"}</span>
    {#if snapshot}<small>{$t("Last sample")} {date.format(new Date(snapshot.updated_at))} · PID {snapshot.pid}</small>{/if}
  </section>
  {#if unavailable || paused}<div class="notice" role="status">{$t(paused ? "Showing the last available sample." : "State unavailable or stale")}</div>{/if}
  {#if snapshot && totals}
    {#if mode === "configuration"}
      <section class="panel"><h2>{$t("Indexing configuration")}</h2>
        <dl class="metrics">
          <div><dt>{$t("Workers")}</dt><dd>{snapshot.configuration.workers}</dd></div>
          <div><dt>{$t("CPU affinity")}</dt><dd>{snapshot.configuration.cpu_affinity}</dd></div>
          <div><dt>{$t("Memory guard")}</dt><dd>{bytes(snapshot.configuration.memory_guard_bytes)}</dd></div>
          <div><dt>{$t("Batch budget")}</dt><dd>{bytes(snapshot.configuration.batch_bytes)}</dd></div>
          <div><dt>{$t("Index output quota")}</dt><dd>{bytes(snapshot.configuration.max_output_bytes)}</dd></div>
          <div><dt>{$t(snapshot.publication.cross_shard_dedup ? "Pending input budget" : "Pilot shard limit")}</dt><dd>{snapshot.configuration.max_shards}</dd></div>
          <div><dt>{$t("Admission interval")}</dt><dd>{snapshot.configuration.admission_seconds} s</dd></div>
          <div><dt>{$t("Worker priority")}</dt><dd>{snapshot.configuration.priority}</dd></div>
        </dl>
        <p>{$t(flow?.indexer_adoption ? "Index adoption confirmed" : "Ranking models and graph signals are not configured for this local pilot.")}</p>
        <p>{$t("The memory guard is sampled; operating system memory quotas remain unverified.")}</p>
        <p>{$t("Input spool")}: <code>{snapshot.source.spool}</code></p>
      </section>
    {:else}
      <section class="indexing-cards" aria-label={$t("Indexing totals")}>
        <article class="panel"><span>{$t(snapshot.publication.cross_shard_dedup ? "Active documents" : "Documents committed")}</span><strong>{number(totals.indexed_documents)}</strong><small>{$t(snapshot.publication.cross_shard_dedup ? "Current URL versions" : "Sum of local shards")}</small></article>
        <article class="panel"><span>{$t(snapshot.publication.cross_shard_dedup ? "Consumed segments" : "Completed shards")}</span><strong>{number(totals.shards_completed)}</strong><small>{$t("Sealed inputs consumed")}</small></article>
        <article class="panel"><span>{$t(snapshot.publication.cross_shard_dedup ? "Queued segments" : "Pending shards")}</span><strong>{number(totals.shards_pending)}</strong><small>{$t("Admitted within pilot limits")}</small></article>
        <article class="panel"><span>{$t(snapshot.publication.cross_shard_dedup ? "Failed inputs" : "Failed shards")}</span><strong>{number(totals.shards_failed)}</strong><small>{$t("Sources and checkpoints retained")}{#if totals.shards_deferred} · {$t("Deferred")}: {totals.shards_deferred}{/if}</small></article>
      </section>
      {#if mode === "overview"}
        <section class="flow-strip" aria-label={$t("Processing and resources")}>
          <a href="#/indexing/shards"><span>{$t("Sealed segments")}</span><strong>{number(snapshot.source.sealed)}</strong></a>
          <span class="flow-arrow">→</span><div><span>{$t("Source records committed")}</span><strong>{number(totals.source_records_completed)}</strong></div>
          <span class="flow-arrow">→</span><div><span>{$t(snapshot.publication.cross_shard_dedup ? "Active documents" : "Documents committed")}</span><strong>{number(totals.indexed_documents)}</strong></div>
          <span class="flow-arrow">→</span><a class="pending-step" href="#/indexing/generations"><span>{$t("Local serving selector")}</span><strong>{$t(generation?.active ? "Ready" : "Pending")}</strong></a>
        </section>
        <section class="chart-grid">
          <article class="panel chart-main"><header><h2>{$t("Documents per minute")}</h2><span>{$t("Last 10 minutes")}</span></header><TimeChart points={throughput} label="Documents per minute" /></article>
          <article class="panel"><header><h2>{$t("Current worker memory")}</h2><strong>{bytes(snapshot.worker?.rss_bytes)}</strong></header><TimeChart points={rss} label="Current worker memory" unit="MiB" color="#59768b" /></article>
          <article class="panel"><header><h2>{$t("Current worker CPU")}</h2><strong>{snapshot.worker ? number(snapshot.worker.cpu_percent) + " %" : $t("Waiting")}</strong></header><TimeChart points={cpu} label="Current worker CPU" unit="%" color="#9d7242" /></article>
        </section>
        <section class="panel work-console">
          <header><h2>{$t(snapshot.worker ? "Active worker" : "Last processed URL")}</h2><span class:working={!!snapshot.worker}>{snapshot.worker ? $t(stageNames[snapshot.runtime?.stage ?? ""] ?? states[snapshot.runtime?.stage ?? ""] ?? "Indexing") : $t(reasons[snapshot.reason] ?? snapshot.reason)}</span></header>
          <div class="console-path"><code>{snapshot.runtime?.current_url ?? snapshot.worker?.input ?? snapshot.recent_events.at(-1)?.message ?? "—"}</code></div>
          {#if snapshot.worker}<progress value={snapshot.worker.records_committed} max={Math.max(1,snapshot.worker.records_total)}></progress><small>{snapshot.worker.records_committed} / {snapshot.worker.records_total} · {number(snapshot.worker.elapsed_seconds)} s</small>{/if}
          <div class="work-footer">{#if snapshot.runtime?.recovered_reference_inputs}<span>{$t("Recovered references")} <b>{number(snapshot.runtime.recovered_reference_inputs)}</b></span>{/if}<span>{$t("CPU milliseconds per document")} <b>{number(cpuPerDocument)}</b></span><span>{$t("Output and retained work")} <b>{bytes(totals.output_bytes)}</b></span><a href="#/indexing/shards">{$t("Inspect shards")} →</a></div>
        </section>
        <section class="panel coexist-strip"><h2>{$t("Crawler and indexer coexistence")}</h2><span>{$t("Crawler CPU")} <b>{number(crawler?.resources?.process_cpu_percent)} %</b></span><span>{$t("Crawler memory")} <b>{bytes(crawler?.resources?.rss_bytes)}</b></span><span>{$t("Spool usage")} <b>{bytes(crawler?.spool?.spool_bytes)}</b></span><span>{$t("Input scan errors")} <b>{number(snapshot.source.scan_errors)}</b></span></section>
        {#if stages.length}<section class="diagnostic-grid">
          <article class="panel"><h2>{$t("Stage CPU time")}</h2><div class="bars">{#each stages as [stage,value]}<div><span>{$t(stageNames[stage] ?? stage)}</span><div class="track"><i style:width={`${value.cpu_ms/stageMax*100}%`}></i></div><b>{number(value.cpu_ms)} ms</b></div>{/each}</div></article>
          <article class="panel"><h2>{$t("Record decisions")}</h2><div class="bars">{#each outcomes as [outcome,value]}<div><span>{$t(outcomeNames[outcome] ?? outcome)}</span><div class="track"><i style:width={`${value/outcomeMax*100}%`}></i></div><b>{number(value)}</b></div>{/each}</div></article>
        </section>{/if}
        {#if snapshot.publication.cross_shard_dedup}<section class="panel preview"><header><h2>{$t("Local search preview")}</h2><small>{$t("Public search publication pending")}</small></header><form onsubmit={event=>{event.preventDefault();searchPreview();}}><input bind:value={previewQuery} aria-label={$t("Search")} placeholder={$t("Search indexed documents")} maxlength="4096" required /><button type="submit">{$t("Search")}</button></form><p aria-live="polite">{previewState ? $t(previewState) : ""}</p>{#each preview as result}<article><strong>{result.title}</strong><code>{result.url}</code><p>{result.snippet}</p></article>{/each}</section>{/if}
        {#if snapshot.runtime?.error}<p class="notice" role="status">{snapshot.runtime.error}</p>{/if}
        <a class="generation-link" href="#/indexing/generations">{$t("Generation catalog")} →</a>
      {:else}
        <section class="panel"><h2>{$t("Input shards")}</h2>
          <div class="filters"><label>{$t("Search")}: <input bind:value={query} placeholder={$t("Segment or error")} /></label>
            <label>{$t("State")}: <select bind:value={filter}><option value="all">{$t("All")}</option>{#each ["pending", "running", "completed", "deferred", "failed"] as state}<option value={state}>{$t(states[state])}</option>{/each}</select></label></div>
          <div class="table-scroll"><table><thead><tr><th>{$t("Segment")}</th><th>{$t("State")}</th><th>{$t("Source records")}</th><th>{$t(snapshot.publication.cross_shard_dedup ? "New current URLs" : "Documents")}</th><th>{$t("Processing time")}</th><th>{$t("Sampled peak RSS")}</th></tr></thead>
            <tbody>{#each rows as row}<tr><td><code>{row.id.slice(0,8)}</code>{#if row.error}<small class="job-error">{row.error}</small>{/if}</td><td>{$t(states[row.state] ?? row.state)}</td><td>{row.cursor} / {row.records}</td><td>{row.state === "completed" ? number(row.documents) : "—"}</td><td>{number(row.elapsed_seconds)} s</td><td>{row.peak_rss_bytes ? bytes(row.peak_rss_bytes) : "—"}</td></tr>{/each}</tbody>
          </table></div>
          <Pagination {page} pages={Math.max(1, Math.ceil(filtered.length / 10))} total={filtered.length} pageSize={10} onpage={value => page = value} />
        </section>
      {/if}
      {#if mode !== "overview" && snapshot.worker}<section class="panel"><h2>{$t("Active worker")}</h2><p><code>{snapshot.worker.input}</code> · PID {snapshot.worker.pid}</p><p>{snapshot.worker.records_committed} / {snapshot.worker.records_total} {$t("Source records committed")} · {number(snapshot.worker.elapsed_seconds)} s</p></section>{/if}
    {/if}
  {/if}
</div>

<style>
  .diagnostic-grid {display:grid;grid-template-columns:1fr 1fr;gap:16px;}
  .bars {display:grid;gap:16px;margin-top:22px;}.bars>div {display:grid;grid-template-columns:130px 1fr 85px;gap:12px;align-items:center;font-size:12px;}.bars b {margin:0;text-align:right;}
  .track {height:6px;background:#edf0ed;border-radius:2px;}.track i {display:block;height:100%;background:#367b62;border-radius:2px;}
  .preview header {display:flex;justify-content:space-between;gap:12px;flex-wrap:wrap;}.preview form {display:flex;gap:12px;margin-top:18px;}.preview input {flex:1;min-width:0;}.preview button {border:0;background:#256a55;color:white;padding:8px 18px;border-radius:4px;}.preview article {padding:16px 0;border-top:1px solid #e5e9e6;}.preview code {display:block;margin-top:5px;}
  .chart-grid { display: grid; grid-template-columns: 2fr 1fr 1fr; gap: 16px; }
  .chart-grid header, .work-console header { display: flex; justify-content: space-between; gap: 14px; align-items: center; margin-bottom: 12px; }
  header span, header strong { font-size: 12px; color: #5a6b62; }
  .flow-strip { display: flex; align-items: center; padding: 18px 24px; border: 1px solid #dde5de; border-radius: 6px; background: #f8faf8; gap: 24px; }
  .flow-strip > div, .flow-strip > a { flex: 1; color: inherit; } .flow-strip span { font-size: 11px; color: #67796d; } .flow-strip strong { display: block; margin-top: 7px; font-size: 21px; font-variant-numeric: tabular-nums; }
  .flow-arrow { font-size: 19px !important; } .pending-step strong { font-size: 15px; color: #78827a; }
  .console-path { padding: 14px; background: #f4f7f4; border-radius: 4px; color: #384b3d; }
  progress { width: 100%; height: 6px; accent-color: #256a55; margin-top: 18px; }
  .work-footer, .coexist-strip { display: flex; align-items: center; gap: 24px; flex-wrap: wrap; font-size: 12px; }
  .work-footer { margin-top: 18px; } .work-footer a { margin-left: auto; } b { font-weight: 600; margin-left: 8px; }
  .working { color: #217454; } .generation-link {font-size:13px;color:#367b62;justify-self:start;}
  .indexing-content { display: grid; gap: 20px; min-width: 0; }
  .panel { padding: 22px 24px; min-width: 0; }
  .indexing-health { display: flex; align-items: center; justify-content: space-between; gap: 16px; flex-wrap: wrap; }
  h2 { font-size: 15px; margin: 0; } p, small, dt { color: #65748a; font-size: 12px; line-height: 1.7; } p { margin-bottom: 0; }
  .indexing-health > span { background: #e7f5ed; color: #167044; border-radius: 6px; padding: 7px 10px; font-size: 12px; }
  .indexing-health > span.warning { background: #fff3df; color: #945300; }
  .indexing-cards { display: grid; grid-template-columns: repeat(4,minmax(0,1fr)); gap: 16px; }
  .indexing-cards span, .indexing-cards small { display: block; font-size: 12px; }
  .indexing-cards strong { display: block; font-size: 30px; margin: 12px 0; }
  .metrics { display: grid; grid-template-columns: repeat(4,minmax(0,1fr)); gap: 20px; margin: 22px 0; }
  dd { margin: 6px 0 0; font-size: 19px; font-variant-numeric: tabular-nums; }
  .filters { display: flex; gap: 18px; flex-wrap: wrap; margin: 22px 0; }
  .filters label { font-size: 12px; } input, select { max-width: 100%; border: 1px solid #d5dce6; padding: 8px; border-radius: 4px; }
  .table-scroll { overflow-x: auto; } table { border-collapse: collapse; width: 100%; font-size: 12px; text-align: left; }
  th, td { border-bottom: 1px solid #e3e9f0; padding: 12px; vertical-align: top; }
  code { overflow-wrap: anywhere; font-size: 12px; } .job-error { display: block; max-width: 260px; color: #a34b12; }
  @media(max-width:1100px) { .chart-grid { grid-template-columns: 1fr 1fr; } .chart-main { grid-column: 1/-1; } .flow-strip { gap: 10px; flex-wrap: wrap; } .indexing-cards, .metrics { grid-template-columns: repeat(2,minmax(0,1fr)); } }
  @media(max-width:540px) { .diagnostic-grid {grid-template-columns:1fr;}.bars>div {grid-template-columns:100px 1fr 65px;gap:8px;} .chart-grid { grid-template-columns: 1fr; } .flow-strip > div, .flow-strip > a { min-width: 90px; } .flow-arrow { display: none; } .panel { padding: 18px; } .indexing-cards { gap: 10px; } .metrics { gap: 14px; } }
</style>
