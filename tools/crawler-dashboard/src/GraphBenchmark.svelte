<script lang="ts">
  import { onMount } from "svelte";
  import { getI18n } from "./i18n";
  const {t,formatLocale}=getI18n();
  type Statistic = {median:number;mad:number};
  type Measurement = Record<string,Statistic>;
  let report=$state<{binary_sha256:string;corpus_sha256:string;summaries:Record<string,Measurement>;incremental:{summary:Record<string,Measurement>;changed_fraction:number};passed:boolean}|null>(null);
  const format=$derived(new Intl.NumberFormat($formatLocale,{maximumFractionDigits:1}));
  const value=(v?:number)=>v===undefined?'—':format.format(v);
  const cases=['small','medium','medium-unique','adversarial','quota'];
  const maximum=$derived(Math.max(1,...cases.flatMap(c=>['legacy','versioned'].map(mode=>report?.summaries[`separate_cpu/${c}/${mode}`]?.rss_bytes?.median??0))));
  onMount(async ()=>{try{const response=await fetch('/webgraph-benchmark.json',{cache:'no-store'});if(response.ok)report=await response.json();}catch{/* Optional benchmark output never affects operation. */}});
</script>

{#if report}
<section class="panel graph-benchmark">
  <header><h2>{$t("Webgraph")} · {$t("Pruebas de capacidad")}</h2><span>{$t("Tres repeticiones")} · median / MAD</span></header>
  <div class="legend"><span><i></i> {$t("Before")}</span><span><i class="after"></i> {$t("After")}</span><span>RSS · MiB</span></div>
  <div class="comparison">{#each cases as name}<div><code>{name}</code><div>{#each ['legacy','versioned'] as mode}{@const measurement=report.summaries[`separate_cpu/${name}/${mode}`]?.rss_bytes}<span><i class:after={mode==='versioned'} style:width={`${(measurement?.median??0)/maximum*70}%`}></i><b>{value((measurement?.median??0)/1048576)} ± {value((measurement?.mad??0)/1048576)}</b></span>{/each}</div></div>{/each}</div>
  <p>{$t("Versioned durability and relationship labels add work; gains depend on the cohort.")}</p>
  <div class="table-wrap"><table><thead><tr><th>{$t("Generation")}</th><th>CPU · ms</th><th>{$t("Written bytes")} · MiB</th></tr></thead><tbody>{#each ['rebuild','incremental'] as mode}{@const measurement=report.incremental.summary[mode]}<tr><td><code>{mode}</code> · {value(report.incremental.changed_fraction*100)} %</td><td>{value(measurement.cpu_ms.median)} ± {value(measurement.cpu_ms.mad)}</td><td>{value(measurement.written_bytes.median/1048576)} ± {value(measurement.written_bytes.mad/1048576)}</td></tr>{/each}</tbody></table></div>
  <details><summary>{$t("Build and scope")}</summary><code>{report.binary_sha256}</code><code>{report.corpus_sha256}</code></details>
</section>
{/if}

<style>
  .graph-benchmark{margin-bottom:22px;padding:24px;min-width:0}header,.legend{display:flex;justify-content:space-between;gap:20px;flex-wrap:wrap}h2{font-size:16px;margin:0}header>span,.legend,p,summary{font-size:12px;color:#68756d}.legend{justify-content:flex-start;margin:26px 0 20px}.legend i{display:inline-block;width:9px;height:9px;margin-right:4px;background:#9cacb6}.legend i.after{background:#367b62}.comparison{display:grid;gap:18px}.comparison>div{display:grid;grid-template-columns:130px 1fr;align-items:center;gap:20px}.comparison code{font-size:12px}.comparison>div>div{display:grid;gap:5px}.comparison span{display:flex;gap:12px;align-items:center}.comparison i{height:8px;background:#9cacb6;min-width:1px;max-width:70%}.comparison i.after{background:#367b62}.comparison b{white-space:nowrap;font-size:11px;font-variant-numeric:tabular-nums}p{margin:24px 0;max-width:80ch;line-height:1.6}.table-wrap{overflow:auto}table{border-collapse:collapse;width:100%;font-size:12px;text-align:left}th,td{padding:12px 8px;border-bottom:1px solid #e2e7e2;font-variant-numeric:tabular-nums}details{margin-top:20px}details code{display:block;font-size:11px;overflow-wrap:anywhere;margin-top:12px}@media(max-width:540px){.comparison>div{grid-template-columns:95px 1fr;gap:10px}.graph-benchmark{padding:18px}.comparison b{font-size:10px}}
</style>
