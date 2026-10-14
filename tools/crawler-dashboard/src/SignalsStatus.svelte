<script lang="ts">
 import {getI18n} from './i18n';
 import type {ComponentFlow} from './component-flow';
 const {t,formatLocale}=getI18n();
 let {flow,compact=false}:{flow:ComponentFlow|null;compact?:boolean}=$props();
 const state=$derived(flow?.signals); const signal=$derived(state?.signals);
 const adopted=$derived(!!state && flow?.indexer_adoption?.generation===state.generation && flow.indexer_adoption.manifest_sha256===state.manifest_sha256);
 const fmt=$derived(new Intl.NumberFormat($formatLocale,{maximumFractionDigits:3}));
 const number=(value?:number)=>value===undefined?'—':fmt.format(value);
 const scoreFormat=$derived(new Intl.NumberFormat($formatLocale,{maximumSignificantDigits:4}));
 const max=$derived(Math.max(.000001,...(signal?.top??[]).map(v=>v.score)));
</script>
<section class="panel signal-status" aria-label={$t('Ranking signals')}>
 <header><div><h2>{$t('Ranking signals')}</h2><span>{$t(signal?'Exact harmonic centrality':'No confirmed scores yet')}{signal?` · ${signal.scope}`:''}</span></div><a href="#/indexing/generations" class:adopted>{$t(adopted?'Index adoption confirmed':'Index adoption pending')} →</a></header>
 {#if signal && state}
 <div class="signal-metrics"><div><small>{$t('Nodes')}</small><strong>{number(signal.nodes)}</strong></div><div><small>{$t('Eligible links')}</small><strong>{number(signal.edges)}</strong></div><div><small>{$t('Iterations')}</small><strong>{number(signal.iterations)}</strong></div><div><small>{$t('Score CPU')}</small><strong>{number(signal.cpu_ms)} <em>ms</em></strong></div></div>
 {#if !compact}
 <div class="rank-plot" role="img" aria-label={$t('Highest incoming harmonic scores')}>{#each signal.top.slice(0,10) as row}<div><code title={row.node}>{row.node}</code><span class="bar"><i style:width={`${row.score/max*100}%`}></i></span><b>{scoreFormat.format(row.score)}</b></div>{/each}</div>
 <footer><span>{$t('Exact error bound')}: ≤ {signal.absolute_error_bound}</span><span>{$t('Reserved memory')}: {number(signal.reserved_bytes/1048576)} MiB</span><span>{$t('Planner')}: {$t('Compatible · inactive')}</span></footer>
 <details><summary>{$t('Build and scope')}</summary><dl><dt>{$t('Graph generation')}</dt><dd>{state.generation}</dd><dt>{$t('Index signal generation')}</dt><dd>{flow?.indexer_adoption?.generation??'—'}</dd><dt>{$t('Algorithm')}</dt><dd>{signal.algorithm}</dd><dt>{$t('Legacy identity aliases')}</dt><dd>{signal.legacy_collisions}</dd></dl></details>
 {/if}
 {:else}<div class="no-signals">{$t('Scores are calculated only within the configured graph budget.')}</div>{/if}
 {#if flow?.adoption_error}<p class="failure">{flow.adoption_error}</p>{/if}
</section>
<style>.signal-status{padding:24px;min-width:0}header{display:flex;justify-content:space-between;align-items:center;gap:18px;flex-wrap:wrap}h2{font-size:16px;margin:0 0 7px}header span,small,footer,summary,dt{font-size:11px;color:#68756d}header a{font-size:12px;color:#956a20}.adopted{color:#326d50}.signal-metrics{display:grid;grid-template-columns:repeat(4,1fr);gap:20px;padding:25px 0}small,strong{display:block}strong{font-size:26px;margin-top:8px;font-variant-numeric:tabular-nums}em{font-size:12px;font-style:normal;font-weight:400}.rank-plot{display:grid;gap:15px;margin:10px 0 28px}.rank-plot>div{display:grid;grid-template-columns:minmax(120px,1.2fr) 2fr 60px;gap:18px;align-items:center}code{font-size:12px;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.bar{height:8px;background:#edf1ed}.bar i{display:block;height:100%;background:#326d50}b{font-size:12px;font-variant-numeric:tabular-nums;text-align:right}footer{display:flex;gap:22px;flex-wrap:wrap;padding-top:18px;border-top:1px solid #e4e9e4}details{margin-top:20px}dl{display:grid;grid-template-columns:140px 1fr;gap:10px;font-size:12px}dd{margin:0;overflow-wrap:anywhere}.failure{color:#955020;font-size:12px}.no-signals{padding-top:20px;color:#68756d;font-size:12px}@media(max-width:650px){.signal-metrics{grid-template-columns:repeat(2,1fr)}.rank-plot>div{grid-template-columns:minmax(90px,1fr) 1fr 50px;gap:10px}.signal-status{padding:18px}}
</style>
