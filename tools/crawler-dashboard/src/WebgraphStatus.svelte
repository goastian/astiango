<script lang="ts">
  import { getI18n } from "./i18n";
  import WorkPulse from "./WorkPulse.svelte";
  import SignalsStatus from "./SignalsStatus.svelte";
  import type { ComponentFlow } from "./component-flow";
  import TimeChart from "./TimeChart.svelte";
  import Pagination from "./Pagination.svelte";
  import type { WebgraphResponse, Snapshot } from "./types";
  import { consumerActivity } from "./work-status";
  const { t, formatLocale } = getI18n();
  let { flow = null, response, crawler, now, error = "", mode = "overview" }: { flow?:ComponentFlow|null; response: WebgraphResponse | null; crawler?: Snapshot; now: number; error?: string; mode?: string } = $props();
  const snapshot = $derived(response?.snapshot);
  const totals = $derived(snapshot?.totals);
  const activity = $derived(consumerActivity(snapshot,crawler));
  const current = $derived(snapshot?.scope === "current_version_graph");
  const generation = $derived(snapshot?.generation);
  const runtime = $derived(snapshot?.runtime);
  const stageNames: Record<string, string> = {read_decode:"Read and decode",dom:"DOM",links:"Extracting links",dedup_prepare:"Deduplicación exacta",ledger_commit:"Commit",writer_commit:"Writer",merge:"Merge",validation:"Validation",centrality_iterations:"Centrality",score_persistence:"Persisting scores"};
  const stages = $derived(Object.entries(runtime?.stages ?? {}).filter(([,v]) => v.cpu_ms > 0));
  const stageMax = $derived(Math.max(1,...stages.map(([,v]) => v.cpu_ms)));
  const decisions = $derived(Object.entries(runtime?.outcomes ?? {}));
  const decisionTotal = $derived(Math.max(1,decisions.reduce((total,[,count]) => total+count,0)));
  const alive = $derived(!!snapshot && !error && response?.meta.process_alive === true && now-Date.parse(snapshot.updated_at) <= 10_000);
  const fmt = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 2 }));
  const number = (v?: number) => v === undefined ? "—" : fmt.format(v);
  const bytes = (v?: number) => v === undefined ? "—" : `${number(v/1048576)} MiB`;
  const names: Record<string, string> = { idle: "Waiting", publishing:"Preparing generation", deferred:"Deferred", generation:"Generations", extracting: "Extracting links", centrality: "Calculating scores", blocked: "Backpressure", draining: "Draining", stopped: "Stopped", pending: "Pending", running: "Extracting links", graph_committed: "Graph committed", calculating: "Calculating scores", completed: "Committed", failed: "Failed", calculated: "Calculated", empty: "Empty graph", budget: "Centrality budget reached" };
  const reasons: Record<string, string> = { waiting_for_segments: "Waiting for sealed segments", admission_interval: "Admission interval", output_quota: "Graph output quota reached", disk_reserve: "Disk reserve reached", source_unavailable: "Input spool unavailable", processing_segment: "Processing sealed segment", shutdown: "Stopped" };
  let page = $state(1);
  let filter = $state("all");
  let query = $state("");
  $effect(() => { filter; query; page = 1; });
  const filtered = $derived((snapshot?.inputs ?? []).filter(v => (filter === "all" || v.state === filter) && `${v.id} ${v.file} ${v.error ?? ""}`.toLocaleLowerCase().includes(query.toLocaleLowerCase())));
  const oldestVisible = $derived(Math.min(Infinity,...(snapshot?.inputs ?? []).filter(v=>["pending","running","deferred"].includes(v.state)).map(v=>v.sealed_at*1000)));
  const pages = $derived(Math.max(1, Math.ceil(filtered.length/10)));
  const currentPage = $derived(Math.min(page, pages));
  const rows = $derived(filtered.slice((currentPage-1)*10, currentPage*10));
  const rate = $derived.by(() => {
    const rows = (snapshot?.history ?? []).filter(v => now-Date.parse(v.at) <= 60_000);
    if (rows.length < 2) return undefined;
    const seconds = (Date.parse(rows[rows.length-1].at)-Date.parse(rows[0].at))/1000;
    return seconds > 0 ? Math.max(0, (rows[rows.length-1].edges-rows[0].edges)*60/seconds) : undefined;
  });
</script>

<div class="graph-view">
  <section class="graph-health">
    <div><h2>{$t("Webgraph")}</h2><span>{$t(snapshot ? activity.detail : "Webgraph state is unavailable.")}</span></div>
    <span class="state" class:warning={!alive || activity.warning}>{$t(alive && snapshot ? activity.label : "State unavailable or stale")}</span>
  </section>
  {#if snapshot && totals}
    {#if mode === "configuration"}
      <section class="panel"><h2>{$t("Graph configuration")}</h2><dl class="limits">
        <div><dt>{$t("Workers")}</dt><dd>{snapshot.configuration.workers}</dd></div>
        <div><dt>{$t("Worker priority")}</dt><dd>{snapshot.configuration.priority}</dd></div>
        <div><dt>{$t("Memory guard")}</dt><dd>{bytes(snapshot.configuration.memory_guard_bytes)}</dd></div>
        <div><dt>{$t("Graph output quota")}</dt><dd>{bytes(snapshot.configuration.max_output_bytes)}</dd></div>
        <div><dt>{$t("Disk reserve")}</dt><dd>{bytes(snapshot.configuration.minimum_disk_free_bytes)}</dd></div>
        <div><dt>{$t("Input size budget")}</dt><dd>{bytes(snapshot.configuration.max_input_bytes)}</dd></div>
        <div><dt>{$t("Pending input budget")}</dt><dd>{snapshot.configuration.max_pending}</dd></div>
        <div><dt>{$t("Centrality edge budget")}</dt><dd>{number(snapshot.configuration.max_centrality_edges)}</dd></div>
      </dl><details><summary>{$t("Build and scope")}</summary><code>{snapshot.configuration.binary_sha256}</code><p>{$t(current ? "Exact capture ledger; compatible graph generations. Signal adoption is separate." : "Per-input graphs. Global deduplication and signal adoption are planned separately.")}</p></details></section>
    {:else if mode === "generations"}
      <section class="panel generation"><header><h2>{$t("Active generation")}</h2><span class="state">{$t(generation ? "Ready" : "Preparing")}</span></header>
      {#if generation}
        <code class="generation-id">{generation.id}</code>
        <div class="generation-stats"><div><span>{$t("Current URL versions")}</span><strong>{number(generation.documents)}</strong></div><div><span>{$t("Stored links")}</span><strong>{number(generation.edges)}</strong></div><div><span>{$t("Reused partitions")}</span><strong>{number(generation.reused_buckets)} / {generation.partitions.length}</strong></div></div>
        <div class="partition-grid" aria-label={$t("Graph partitions")}>{#each generation.partitions as partition}<div title={`${partition.bucket}: ${partition.edges}`} class:populated={partition.edges > 0}><span>{partition.bucket}</span><b>{number(partition.edges)}</b></div>{/each}</div>
        <details><summary>{$t("Build and scope")}</summary><code>{generation.logical_sha256}</code><code>{generation.build_id}</code><p>{$t("Previous generation retained")}.</p></details>
      {:else}<p>{$t("No validated generation yet")}</p>{/if}
      {#if snapshot.publication_error}<p class="failure">{snapshot.publication_error}</p>{/if}</section>
    {:else if mode === "inputs"}
      <section class="panel"><header><h2>{$t("Graph inputs")}</h2><span>{$t("Oldest visible pending input")}: {Number.isFinite(oldestVisible) ? `${number(Math.max(0,(now-oldestVisible)/1000))} s` : "—"} · {$t("Outside input budget")}: {number(snapshot.source.oversized)}</span></header>
        <div class="filters"><label>{$t("Search")} <input bind:value={query} /></label><label>{$t("State")} <select bind:value={filter}><option value="all">{$t("All")}</option>{#each ["pending","running","deferred","graph_committed","calculating","completed","failed"] as state}<option value={state}>{$t(names[state])}</option>{/each}</select></label></div>
        <div class="table-wrap"><table><thead><tr><th>{$t("Segment")}</th><th>{$t("State")}</th><th>{$t("Source records")}</th><th>{$t(current ? "Graph checkpoint" : "Stored links")}</th><th>{$t("Centrality")}</th><th>{$t("Processing time")}</th></tr></thead><tbody>{#each rows as row}<tr><td><code>{row.id.slice(0,8)}</code>{#if row.error}<small class="failure">{row.error}</small>{/if}</td><td>{$t(names[row.state] ?? row.state)}</td><td>{number(row.records)}</td><td>{number(row.edges)}</td><td>{$t(names[row.score_state] ?? row.score_state)}</td><td>{number(row.elapsed_seconds)} s</td></tr>{/each}</tbody></table></div>
        <Pagination page={currentPage} {pages} total={filtered.length} pageSize={10} onpage={v => page=v} />
      </section>
    {:else if mode === "centrality"}
      <SignalsStatus {flow} />
    {:else}
      <WorkPulse {flow} component="webgraph" />
      <section class="graph-summary"><div class="link-total"><span>{$t("Stored links")}</span><strong>{number(totals.stored_edges)}</strong><small>{$t(current ? "Current graph versions" : "Per-input graphs; not global unique links")}</small></div><div class="summary-counts"><div><span>{$t("Consumed segments")}</span><strong>{number(totals.completed)}</strong></div><div><span>{$t("Queued segments")}</span><strong>{number(totals.pending)}</strong></div><div><span>{$t("Failed inputs")}</span><strong>{number(totals.failed)}</strong></div></div></section>
      <section class="graph-flow" aria-label={$t("Graph pipeline")}><a href="#/webgraph/inputs"><span>01</span><b>{$t("Source records")}</b><strong>{number(totals.source_records)}</strong></a><div><span>02</span><b>{$t("Stored links")}</b><strong>{number(totals.stored_edges)}</strong></div><a href={current ? "#/webgraph/generations" : "#/webgraph/centrality"}><span>03</span><b>{$t(current ? "Active generation" : "Inputs with scores")}</b><strong>{current ? $t(generation ? "Ready" : "Preparing") : number(totals.scored_inputs)}</strong></a><a href="#/webgraph/centrality"><span>04</span><b>{$t("Signal adoption")}</b><strong>{$t(flow?.indexer_adoption?.generation === flow?.signals?.generation && flow?.signals ? "Confirmed" : "Pending")}</strong></a></section>
      <section class="graph-charts"><article class="panel"><header><h2>{$t("Stored links")}</h2><span>{number(rate)} / min</span></header><TimeChart points={snapshot.history.map(v => ({at:v.at,value:v.edges}))} label="Stored links" /></article><article class="panel"><header><h2>{$t("Graph memory")}</h2><span>{bytes(snapshot.worker?.rss_bytes)}</span></header><TimeChart points={snapshot.history.map(v => ({at:v.at,value:v.rss_mib}))} label="Graph memory" unit="MiB" color="#596f87" /></article></section>
      {#if current}
        <section class="graph-charts diagnostics"><article class="panel"><header><h2>{$t("Stage CPU")}</h2><span>{$t("Latest segment")} · ms</span></header><div class="stage-bars">{#each stages as [name,measurement]}<div><span>{$t(stageNames[name] ?? name)}</span><div class="track"><i style:width={`${measurement.cpu_ms/stageMax*100}%`}></i></div><b>{number(measurement.cpu_ms)}</b></div>{/each}</div></article><article class="panel"><header><h2>{$t("Record decisions")}</h2><span>{number(decisionTotal)}</span></header><div class="decision-bar">{#each decisions as [name,count],i}<i title={`${name}: ${count}`} style:width={`${count/decisionTotal*100}%`} style:background={['#367b62','#7b99af','#dab575','#9b6c78'][i%4]}></i>{/each}</div><div class="decision-list">{#each decisions as [name,count]}<div><code>{name}</code><b>{number(count)}</b></div>{/each}</div></article></section>
        <a class="generation-link panel" href="#/webgraph/generations"><span>{$t("Active generation")}</span><strong>{$t(generation ? "Ready" : "Preparing")}</strong><code>{generation?.id.slice(0,8) ?? "—"}</code><b>{number(generation?.edges)} {$t("Stored links")}</b><span>→</span></a>
      {/if}
      <SignalsStatus {flow} compact />
      <section class="panel work"><header><h2>{$t("Active worker")}</h2><span>{$t(snapshot.worker ? names[snapshot.worker.stage] ?? snapshot.worker.stage : activity.label)}</span></header><code>{snapshot.worker?.input ?? snapshot.latest_input ?? "—"}</code><div><span>{$t("Output and retained work")} <b>{bytes(totals.output_bytes)}</b></span><span>{$t("Input scan errors")} <b>{number(snapshot.source.scan_errors)}</b></span><span>{$t("Read and decode")} <b>{bytes(snapshot.worker?.read_bytes)}</b></span><span>{$t("Written bytes")} <b>{bytes(snapshot.worker?.written_bytes)}</b></span><a href="#/webgraph/inputs">{$t("Graph inputs")} →</a></div></section>
    {/if}
  {:else}<section class="panel"><h2>{$t("Webgraph state is unavailable.")}</h2></section>{/if}
</div>

<style>
  .stage-bars{display:grid;gap:14px;margin-top:22px}.stage-bars>div{display:grid;grid-template-columns:120px 1fr 60px;gap:14px;align-items:center}.stage-bars b,.decision-list b{font-size:12px;text-align:right;font-variant-numeric:tabular-nums}.decision-bar{display:flex;height:12px;margin:24px 0 18px;background:#e9efea}.decision-list{display:grid;gap:10px}.decision-list>div{display:flex;justify-content:space-between;gap:10px}.decision-list code{font-size:11px}.generation-link{display:flex;align-items:center;gap:20px;text-decoration:none;color:#326d50}.generation-link>b{margin-left:auto;font-size:12px}.generation-link>strong{font-size:20px}.generation-id{display:block;margin:24px 0;overflow-wrap:anywhere}.generation-stats{display:grid;grid-template-columns:repeat(3,1fr);gap:20px}.generation-stats strong{display:block;font-size:30px;margin-top:12px}.partition-grid{display:grid;grid-template-columns:repeat(8,1fr);gap:5px;margin:28px 0}.partition-grid>div{background:#eff2ee;padding:9px 7px;border:1px solid #e0e7df;min-width:0}.partition-grid>div.populated{background:#e1eee4}.partition-grid span,.partition-grid b{display:block;font-size:10px;font-variant-numeric:tabular-nums}.partition-grid b{margin-top:5px;font-size:12px;overflow-wrap:anywhere}@media(max-width:540px){.stage-bars>div{grid-template-columns:95px 1fr 40px;gap:8px}.generation-link{flex-wrap:wrap;gap:12px}.generation-stats strong{font-size:22px}.partition-grid{grid-template-columns:repeat(4,1fr)}}
  .graph-view{display:grid;gap:22px;min-width:0}.graph-health,header{display:flex;justify-content:space-between;align-items:center;gap:16px;flex-wrap:wrap}h2{font-size:16px;margin:0}span,small,dt,p,summary{font-size:12px;color:#68756d}.graph-health>div span{display:block;margin-top:7px}.state{padding:7px 10px;background:#e7f2e9;color:#226747;border-radius:4px}.state.warning{background:#fff2dc;color:#90570e}.panel{padding:24px;min-width:0}.graph-summary{display:grid;grid-template-columns:1.2fr 2fr;gap:32px;padding:22px 0;border-bottom:1px solid #dce4de}.link-total>span,.link-total small{display:block}.link-total>strong{display:block;font-size:54px;letter-spacing:-2px;margin:8px 0;font-variant-numeric:tabular-nums}.summary-counts{display:grid;grid-template-columns:repeat(3,1fr);align-items:center;gap:20px}.summary-counts strong{display:block;font-size:30px;margin-top:12px;font-variant-numeric:tabular-nums}.graph-flow{display:grid;grid-template-columns:repeat(4,1fr);gap:1px;border:1px solid #dce4de;background:#dce4de}.graph-flow>a,.graph-flow>div{padding:20px;background:#f7faf7;color:inherit;text-decoration:none}.graph-flow b,.graph-flow strong{display:block;margin-top:10px}.graph-flow b{font-size:12px;font-weight:500}.graph-flow strong{font-size:22px;font-variant-numeric:tabular-nums}.graph-charts{display:grid;grid-template-columns:2fr 1fr;gap:18px}.work>code{display:block;margin:16px 0;padding:12px;background:#f2f6f2;overflow-wrap:anywhere}.work>div,.work a{margin-left:auto;font-size:12px;color:#326f56}.work b{margin-left:8px;font-variant-numeric:tabular-nums}.limits{display:grid;grid-template-columns:repeat(4,1fr);gap:25px;margin:24px 0}.limits dd{margin:8px 0 0;font-size:21px}.filters{display:flex;gap:18px;flex-wrap:wrap;margin:22px 0}.filters label{font-size:12px}input,select{padding:8px;border:1px solid #d8e0d9;border-radius:4px}.table-wrap{overflow:auto}table{width:100%;border-collapse:collapse;text-align:left;font-size:12px}th,td{padding:13px;border-bottom:1px solid #e2e7e2;vertical-align:top}.failure{display:block;max-width:250px;overflow-wrap:anywhere;color:#92500c}.track{height:7px;background:#ecf0eb}.track i{display:block;height:100%;background:#367b62}a:focus-visible{outline:2px solid #367b62;outline-offset:4px}.graph-flow>a:hover{background:#eaf2e9}@media(max-width:900px){.graph-summary,.graph-charts{grid-template-columns:1fr}.limits{grid-template-columns:repeat(2,1fr)}.graph-flow{grid-template-columns:repeat(2,1fr)}}@media(max-width:540px){.panel{padding:18px}.summary-counts{gap:10px}.summary-counts strong{font-size:24px}.link-total>strong{font-size:44px}}
</style>
