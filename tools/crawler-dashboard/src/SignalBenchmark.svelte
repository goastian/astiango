<script lang="ts">
 import {onMount} from 'svelte';import {getI18n} from './i18n';
 const {t,formatLocale}=getI18n();
 type Stats={median:number;mad:number};
 type Cohort={reference_cpu_ms:Stats;optimized_cpu_ms:Stats;cpu_reduction_percent:number};
 let report=$state<{passed:boolean;performance:{synthetic:Cohort;real:Cohort|null;runs:Array<{approximations:Record<string,{relative_p95:number;top10_overlap:number}>}>}}|null>(null);
 const fmt=$derived(new Intl.NumberFormat($formatLocale,{maximumFractionDigits:2}));
 onMount(async()=>{try{const r=await fetch('/webgraph-signals-benchmark.json');if(r.ok)report=await r.json();}catch{}});
</script>
{#if report?.passed}
<section class="panel benchmark-signals"><header><h2>{$t('Exact harmonic centrality')}</h2><span>{$t('Tres repeticiones')} · median / MAD · CPU ms</span></header>
<div class="cohorts">{#each [{name:'synthetic',data:report.performance.synthetic},{name:'real',data:report.performance.real}] as cohort}{#if cohort.data}<article><header><code>{cohort.name}</code><b>{fmt.format(cohort.data.cpu_reduction_percent)}% ↓</b></header>{#each [{name:'BFS',measurement:cohort.data.reference_cpu_ms},{name:'Bitsets',measurement:cohort.data.optimized_cpu_ms}] as row}<div class="measurement"><span>{row.name}</span><div><i style:width={`${row.measurement.median/Math.max(1,cohort.data.reference_cpu_ms.median)*100}%`}></i></div><strong>{fmt.format(row.measurement.median)} ± {fmt.format(row.measurement.mad)}</strong></div>{/each}</article>{/if}{/each}</div>
<details><summary>{$t('Algorithm')} · HLL</summary><div class="table-wrap"><table><thead><tr><th>HLL</th><th>{$t('Relative error · p95')}</th><th>Top-10</th></tr></thead><tbody>{#each Object.entries(report.performance.runs[0].approximations) as [name,row]}<tr><td>{name}</td><td>{fmt.format(row.relative_p95*100)}%</td><td>{fmt.format(row.top10_overlap*100)}%</td></tr>{/each}</tbody></table></div></details>
</section>
{/if}
<style>.benchmark-signals{padding:24px;margin-bottom:22px}header{display:flex;justify-content:space-between;align-items:center;gap:20px;flex-wrap:wrap}h2{font-size:16px;margin:0}header>span,summary{font-size:11px;color:#68756d}.cohorts{display:grid;grid-template-columns:1fr 1fr;gap:30px;margin:25px 0}.cohorts article{min-width:0}.cohorts header{font-size:12px;margin-bottom:20px}.cohorts b{font-size:22px;color:#326d50}.measurement{display:grid;grid-template-columns:50px 1fr 120px;gap:12px;align-items:center;margin:16px 0;font-size:11px}.measurement>div{height:8px;background:#edf1ed;overflow:hidden}.measurement i{display:block;height:100%;background:#326d50}.measurement strong{text-align:right;font-variant-numeric:tabular-nums}details{border-top:1px solid #e4e9e4;padding-top:16px}.table-wrap{overflow:auto}table{width:100%;font-size:12px;border-collapse:collapse;text-align:left}th,td{padding:12px;border-bottom:1px solid #e4e9e4}@media(max-width:850px){.cohorts{grid-template-columns:1fr}.measurement{grid-template-columns:45px 1fr 100px;gap:8px}}
</style>
