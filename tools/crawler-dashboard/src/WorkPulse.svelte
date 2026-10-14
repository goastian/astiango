<script lang="ts">
 import {getI18n} from './i18n';
 import type {ComponentFlow} from './component-flow';
 const {t,formatLocale}=getI18n();
 let {flow,component}:{flow:ComponentFlow|null;component:'indexer'|'webgraph'}=$props();
 const fmt=$derived(new Intl.NumberFormat($formatLocale,{maximumFractionDigits:1}));
 const key=$derived(component==='indexer'?'index_records':'graph_records');
</script>
{#if flow}
<section class="work-pulse" aria-label={$t('Live processing')}>
 <div><span>{$t('Processed records')}</span><strong>{fmt.format(flow.counts[key]??0)}</strong></div>
 <div><span>{$t('Records / min · last 60 s')}</span><strong>{fmt.format(flow.rates[key]??0)}</strong></div>
 <div><span>{$t('Closed backlog · records')}</span><strong>{fmt.format(flow.backlog[component]??0)}</strong></div>
 <div><span>{$t('Next segment')}</span><strong>{flow.open_segment ? `${Math.ceil(flow.open_segment.estimated_seconds_to_seal)} s` : '—'}</strong><small>{flow.open_segment ? `${fmt.format(flow.open_segment.records)} ${$t('Source records')} · ${$t('Estimated')}` : $t('No open segment')}</small></div>
</section>
{/if}
<style>.work-pulse{display:grid;grid-template-columns:repeat(4,1fr);border:1px solid #dce4de;background:white;gap:1px}.work-pulse>div{padding:18px 20px;min-width:0;border-right:1px solid #e5eae5}span,small{display:block;color:#68756d;font-size:11px}strong{display:block;font-size:26px;margin-top:9px;font-variant-numeric:tabular-nums}small{margin-top:5px}@media(max-width:700px){.work-pulse{grid-template-columns:repeat(2,1fr)}}</style>
