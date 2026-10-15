<script lang="ts">
  import { getI18n } from "./i18n";
  import TimeChart from "./TimeChart.svelte";
  import Icon from "./Icon.svelte";
  import type { SearchResponse } from "./search-api";
  const { t, formatLocale } = getI18n();
  let { response = null, mode = "overview", now, error = "" }: {
    response?: SearchResponse | null; mode?: string; now: number; error?: string;
  } = $props();
  const snapshot = $derived(response?.snapshot);
  const live = $derived(!!snapshot && response?.meta.process_alive === true && now-Date.parse(snapshot.updated_at)<10000);
  const format = $derived(new Intl.NumberFormat($formatLocale,{maximumFractionDigits:2}));
  const number = (value?: number) => value === undefined ? "—" : format.format(value);
  let query = $state("");
  let result = $state<{ results?: { url: string; title: string; snippet: string }[]; generation?: string; wall_ms?: number; error?: string; outcome?: string } | null>(null);
  let querying = $state(false);
  let queryError = $state("");
  let actionStatus = $state("");
  let acting = $state(false);
  async function search() {
    querying=true; queryError="";
    try {
      const request=await fetch(`/api/search/query?q=${encodeURIComponent(query)}`,{signal:AbortSignal.timeout(5000)});
      const value=await request.json();
      if(!request.ok) throw Error(value.error ?? value.code ?? request.statusText);
      result=value;
    } catch(reason) { queryError=reason instanceof Error ? reason.message : String(reason); }
    finally { querying=false; }
  }
  async function control(action:string, generation?:string) {
    acting=true;actionStatus="";
    try {
      const request=await fetch("/api/search/control",{method:"POST",headers:{"Content-Type":"application/json"},body:JSON.stringify({id:crypto.randomUUID(),action,generation}),signal:AbortSignal.timeout(5000)});
      const value=await request.json();
      if(!request.ok) throw Error(value.error ?? value.code);
      actionStatus=`${value.action ?? action}: ${value.state} · ${value.id}`;
    } catch(reason) { actionStatus=reason instanceof Error ? reason.message : String(reason); }
    finally { acting=false; }
  }
  function safeUrl(value:string) { try { const url=new URL(value);return ["http:","https:"].includes(url.protocol)?url.href:undefined; } catch { return undefined; } }
</script>

{#if mode === "summary"}
  <a class="search-summary panel" href="#/search/overview"><span><Icon name="search" /><strong>{$t("Search & API")}</strong><small>{$t(live ? snapshot?.ready ? "Ready" : "Waiting for a validated generation" : "State unavailable or stale")}</small></span><b>{number(snapshot?.window.qps)} <small>QPS</small></b><b>{number(snapshot?.window.samples ? snapshot.window.p95_ms : undefined)} <small>p95 · ms</small></b><b>{number(snapshot?.generation?.documents)} <small>{$t("Active documents")}</small></b><Icon name="arrow" /></a>
{:else}
  {#if error || !live}<div class="notice" role="status">{$t("Search state is unavailable or stale.")} {error}</div>{/if}
  <section class="search-kpis" aria-label={$t("Search & API")}>
    <article><span>{$t("Query traffic")}</span><strong>{number(snapshot?.window.qps)}<small>QPS</small></strong><em>{$t("User queries; probes excluded")}</em></article>
    <article><span>{$t("Latency")}</span><strong>{number(snapshot?.window.samples ? snapshot.window.p95_ms : undefined)}<small>p95 · ms</small></strong><em>{number(snapshot?.window.samples)} {$t("Samples")}</em></article>
    <article><span>{$t("Admission")}</span><strong>{number(snapshot?.active)}<small>/ {number(snapshot?.queued)} {$t("In queue")}</small></strong><em>{$t(live ? snapshot?.ready ? "Ready" : "Waiting for a validated generation" : "State unavailable or stale")}</em></article>
    <article><span>{$t("Serving generation")}</span><strong class="generation-id">{snapshot?.generation?.id.slice(0,8) ?? "—"}</strong><em>{number(snapshot?.generation?.documents)} {$t("Active documents")}</em></article>
  </section>
  {#if mode === "overview" || mode === "performance" || mode === "nodes"}
    <section class="search-resources" aria-label={$t("Resources")}>
      <span>CPU <b>{number(snapshot?.resources.cpu_percent)}%</b></span>
      <span>RSS <b>{number(snapshot ? snapshot.resources.rss_bytes/1048576 : undefined)} MiB</b></span>
      <span>FD <b>{number(snapshot?.resources.open_fds)}</b></span>
      <span>{$t("Cache usage")} <b>{number(snapshot ? (snapshot.cache_bytes??0)/1048576 : undefined)} MiB</b></span>
      <span>{$t("Disk reads")} <b>{number(snapshot?.resources.read_bytes===undefined ? undefined : snapshot.resources.read_bytes/1048576)} MiB</b></span>
      <span>{$t("Disk writes")} <b>{number(snapshot?.resources.written_bytes===undefined ? undefined : snapshot.resources.written_bytes/1048576)} MiB</b></span>
    </section>
  {/if}
  {#if snapshot?.last_probe}<p class="query-meta">{$t("Last query check")}: {$t(snapshot.last_probe.state)} · {number(snapshot.last_probe.wall_ms)} ms · {snapshot.last_probe.generation?.slice(0,8)??"—"} · {$t("Probe; not user traffic")}</p>{/if}
  {#if mode === "overview" || mode === "performance"}
    <div class="search-charts"><article class="panel"><header><h2>{$t("Query traffic")}</h2><span>QPS</span></header><TimeChart points={(snapshot?.history??[]).map(p=>({at:p.at,value:p.qps}))} label="Query traffic" color="#326d50" /></article><article class="panel"><header><h2>{$t("Latency")}</h2><span>p95 · ms</span></header><TimeChart points={(snapshot?.history??[]).map(p=>({at:p.at,value:p.p95_ms??NaN}))} label="Latency" color="#596f87" /></article></div>
    {#if mode === "overview"}
      <section class="panel query-panel"><header><h2>{$t("Run query")}</h2><span>{$t("Own index")} · {$t("Probe; not user traffic")}</span></header><form onsubmit={event=>{event.preventDefault();void search();}}><label class="query-label"><span>{$t("Query")}</span><input bind:value={query} maxlength="4096" placeholder={$t("Search the active index")} required /></label><button class="button primary" disabled={querying || !snapshot?.ready}>{$t(querying?"Searching…":"Search")}</button></form>{#if queryError}<p role="alert">{queryError}</p>{/if}{#if result}<div class="query-meta">{$t("Serving generation")}: {result.generation?.slice(0,8)} · {number(result.wall_ms)} ms · {number(result.results?.length)} {$t("Query results")}</div>{#each result.results??[] as page}<article class="query-result"><a href={safeUrl(page.url)} rel="noreferrer" target="_blank">{page.title||page.url}</a><small>{page.url}</small><p>{page.snippet}</p></article>{/each}{#if !result.results?.length}<p>{$t("No results")}</p>{/if}{/if}</section>
    {/if}
    <div class="search-charts"><section class="panel"><header><h2>{$t("Engine work")}</h2><span>{$t("Latest user query")} · ms</span></header>{#each Object.entries(snapshot?.stages_ms??{}) as [name,value]}<div class="stage"><span>{name.replaceAll("_"," ")}</span><meter min="0" max={Math.max(1,...Object.values(snapshot?.stages_ms??{}))} value={value}></meter><b>{number(value)}</b></div>{/each}</section><section class="panel"><header><h2>{$t("Request outcomes")}</h2><span>{$t("User queries; probes excluded")}</span></header>{#each ["complete","no_results","failed","timeouts","cancelled","rejected","cache_hits","cache_misses"] as name}<div class="outcome"><span>{$t(name.replaceAll("_"," "))}</span><b>{number(snapshot?.totals[name])}</b></div>{/each}</section></div>
  {:else if mode === "nodes"}
    <section class="panel table-panel"><header><h2>{$t("Reader readiness")}</h2><span>LocalSearcher · {$t("Own index")}</span></header><div class="table-scroll"><table><thead><tr><th>PID</th><th>{$t("State")}</th><th>{$t("Serving generation")}</th><th>{$t("Load time")}</th><th>{$t("Processed requests")}</th></tr></thead><tbody>{#each snapshot?.nodes??[] as node}<tr><td>{node.pid}</td><td>{$t(node.state)}</td><td>{node.generation.slice(0,8)}</td><td>{number(node.load_ms)} ms</td><td>{number(node.requests)}</td></tr>{/each}</tbody></table></div></section>
  {:else if mode === "generations"}
    <section class="panel table-panel"><header><h2>{$t("Serving generations")}</h2><button class="button" disabled={acting || !snapshot?.previous} onclick={()=>void control("rollback")}>{$t("Rollback generation")}</button></header><p class="query-meta">{$t("Previous generation")}: {snapshot?.previous?.slice(0,8)??"—"} · {$t("Desired generation")}: {snapshot?.desired?.slice(0,8)??"—"}</p><div class="table-scroll"><table><thead><tr><th>{$t("Generation")}</th><th>{$t("State")}</th><th>{$t("Documents")}</th><th>{$t("Actions")}</th></tr></thead><tbody>{#each snapshot?.generations??[] as generation}<tr><td>{generation.id.slice(0,8)}</td><td>{$t(generation.selected?"Serving":generation.state)}</td><td>{number(generation.documents)}</td><td class="generation-actions"><button class="button" disabled={acting || generation.selected} onclick={()=>void control("warm",generation.id)}>{$t("Validate and warm")}</button><button class="button" disabled={acting || generation.selected} onclick={()=>void control("promote",generation.id)}>{$t("Promote generation")}</button></td></tr>{/each}</tbody></table></div>{#if actionStatus}<p role="status">{actionStatus}</p>{/if}</section>
  {:else if mode === "diagnostics"}
    <section class="panel table-panel"><header><h2>{$t("Action status")}</h2></header><div class="table-scroll"><table><thead><tr><th>{$t("Action")}</th><th>{$t("State")}</th><th>{$t("Generation")}</th><th>{$t("Details")}</th></tr></thead><tbody>{#each snapshot?.commands??[] as action}<tr><td>{action.action}</td><td>{$t(action.state)}</td><td>{action.generation?.slice(0,8)??"—"}</td><td>{action.error??action.id}</td></tr>{/each}</tbody></table></div></section><section class="panel table-panel"><header><h2>{$t("Recent events")}</h2></header>{#each snapshot?.events??[] as event}<div class="event"><time>{event.at}</time><b>{event.reason}</b><span>{event.detail}</span></div>{/each}</section>
  {:else if mode === "configuration"}
    <div class="search-charts"><section class="panel"><header><h2>{$t("Search configuration")}</h2></header><dl>{#each Object.entries(snapshot?.configuration??{}) as [key,value]}<div><dt>{key.replaceAll("_"," ")}</dt><dd>{typeof value==="object"?JSON.stringify(value):String(value)}</dd></div>{/each}</dl></section><section class="panel"><header><h2>{$t("Capabilities")}</h2></header><dl>{#each Object.entries(snapshot?.capabilities??{}) as [key,value]}<div><dt>{key.replaceAll("_"," ")}</dt><dd>{typeof value==="object"?JSON.stringify(value):String(value)}</dd></div>{/each}</dl></section></div>
  {:else if mode === "access"}
    <section class="panel access-plan"><header><h2>{$t("Paid API — planned")}</h2><span>{$t("Not enabled")}</span></header><p>{$t("Payments are planned; no charges are enabled.")}</p><div class="access-steps">{#each ["Accounts and plans","Scoped API keys","Durable usage ledger","Payments and reconciliation"] as label,index}<article><span>0{index+1}</span><strong>{$t(label)}</strong><small>{$t("Planned")}</small></article>{/each}</div><p class="query-meta">GET /v1/web/search · {$t("Local access only")}</p></section>
  {/if}
{/if}

<style>
  .search-resources{display:flex;flex-wrap:wrap;gap:16px 28px;padding:0 4px 18px;margin-top:-6px;font-size:11px;color:#68756d}.search-resources b{font-weight:500;color:#344a3c;font-variant-numeric:tabular-nums;margin-left:6px}
  .search-kpis{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));border-block:1px solid #dce4de;margin-bottom:22px}.search-kpis article{padding:22px 20px;border-right:1px solid #dce4de;min-width:0}.search-kpis article:last-child{border:0}.search-kpis article>span{font-size:12px;color:#68756d}.search-kpis strong{display:block;font-size:36px;font-variant-numeric:tabular-nums;margin:14px 0}.search-kpis strong small{font-size:12px;font-weight:400;margin-left:6px}.search-kpis em{font-style:normal;font-size:11px;color:#68756d}.search-kpis strong.generation-id{font-size:28px;line-height:43px}.search-charts{display:grid;grid-template-columns:1fr 1fr;gap:20px;margin-bottom:22px}.panel{padding:24px;min-width:0;margin-bottom:22px}.search-charts .panel{margin:0}header{display:flex;justify-content:space-between;gap:12px;align-items:center;margin-bottom:18px}h2{font-size:15px;margin:0}header>span,.query-meta{font-size:11px;color:#68756d}.query-panel form{display:flex;gap:12px;align-items:end}.query-label{display:grid;gap:8px;flex:1;font-size:12px}.query-label input{width:100%;padding:11px;border:1px solid #cbd5d0;border-radius:3px;background:white;color:inherit}.query-meta{margin:18px 0}.query-result{margin:20px 0;padding-top:16px;border-top:1px solid #e4e9e5}.query-result a{font-weight:600;color:#326d50}.query-result small{display:block;color:#68756d;font-size:11px;margin:6px 0;overflow-wrap:anywhere}.query-result p{font-size:13px;line-height:1.6;margin:8px 0}.stage{display:grid;grid-template-columns:1fr 1fr 60px;gap:12px;font-size:12px;margin:16px 0;align-items:center}.stage b{text-align:right}meter{width:100%;height:8px;accent-color:#326d50}.outcome{display:flex;justify-content:space-between;padding:8px 0;font-size:12px}.table-scroll{overflow-x:auto}table{width:100%;border-collapse:collapse;font-size:12px;text-align:left}th,td{padding:12px 10px;border-bottom:1px solid #e4e9e5}th{font-weight:500;color:#68756d}.generation-actions{display:flex;gap:8px}.generation-actions button{white-space:nowrap;font-size:11px}.event{display:grid;grid-template-columns:180px 150px 1fr;gap:16px;padding:12px 0;font-size:12px;overflow-wrap:anywhere}.event time{font-size:10px;color:#68756d}dl{margin:0}dl>div{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:16px;padding:10px 0;border-bottom:1px solid #edf0ed;font-size:12px}dt{color:#68756d}dd{margin:0;overflow-wrap:anywhere}.access-steps{display:grid;grid-template-columns:repeat(4,1fr);gap:1px;background:#dce4de;border:1px solid #dce4de;margin-top:24px}.access-steps article{padding:22px;background:#fafcf9;display:grid;gap:20px}.access-steps span,.access-steps small{font-size:11px;color:#68756d}.access-steps strong{font-size:13px}.search-summary{display:flex;gap:24px;align-items:center;text-decoration:none;color:inherit}.search-summary>span{display:flex;gap:12px;align-items:center;flex:1;flex-wrap:wrap}.search-summary small{font-size:11px;color:#68756d;font-weight:400}.search-summary>b{font-size:23px;font-variant-numeric:tabular-nums;white-space:nowrap}.search-summary>b small{display:block;margin-top:6px}@media(max-width:1050px){.search-kpis{grid-template-columns:1fr 1fr}.search-kpis article{border-bottom:1px solid #dce4de}.access-steps{grid-template-columns:1fr 1fr}}@media(max-width:700px){.search-charts{grid-template-columns:1fr}.panel{padding:18px}.search-kpis article{padding:18px 12px}.search-kpis strong{font-size:28px}.query-panel form{flex-direction:column;align-items:stretch}.event{grid-template-columns:1fr}.search-summary{flex-wrap:wrap;gap:18px}.search-summary>span{flex-basis:100%}.access-steps{grid-template-columns:1fr}.stage{grid-template-columns:1fr 1fr 45px}}
</style>
