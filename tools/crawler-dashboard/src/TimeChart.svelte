<script lang="ts">
  import { getI18n } from "./i18n";
  const { t, formatLocale } = getI18n();
  let { points, label, unit = "", color = "#256a55" }: { points: Array<{ at: string; value: number }>; label: string; unit?: string; color?: string } = $props();
  let width = $state(720);
  const valid = $derived(points.filter(point => Number.isFinite(point.value) && Number.isFinite(Date.parse(point.at))));
  const max = $derived(Math.max(1, ...valid.map(point => point.value)));
  const start = $derived(valid.length ? Date.parse(valid[0].at) : 0);
  const span = $derived(Math.max(1, valid.length ? Date.parse(valid[valid.length - 1].at) - start : 1));
  const coordinates = $derived(valid.map(point => `${44 + (Date.parse(point.at)-start)/span*(width-70)},${150-point.value/max*124}`).join(" "));
  const format = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 1 }));
  const clock = $derived(new Intl.DateTimeFormat($formatLocale, { hour: "2-digit", minute: "2-digit", hour12: false }));
</script>
<div class="chart" bind:clientWidth={width} role="img" aria-label={`${$t(label)} · ${valid.length ? format.format(valid[valid.length-1].value) : "—"} ${unit}`}>
  <svg viewBox={`0 0 ${Math.max(280,width)} 186`} preserveAspectRatio="none" style:color={color}>
    {#each [0, .5, 1] as tick}<line x1="44" x2={width-26} y1={150-tick*124} y2={150-tick*124} /><text x="34" y={154-tick*124} text-anchor="end">{format.format(max*tick)}</text>{/each}
    {#if valid.length > 1}<polygon points={`44,150 ${coordinates} ${width-26},150`} /><polyline points={coordinates} />{/if}
    {#if valid.length}<text x="44" y="178">{clock.format(new Date(valid[0].at))}</text><text x={width-26} y="178" text-anchor="end">{clock.format(new Date(valid[valid.length-1].at))}</text>{/if}
  </svg>
  {#if valid.length < 2}<span class="empty-chart">{$t("Waiting for samples")}</span>{/if}
</div>
<style>
  .chart { position: relative; height: 188px; } svg { width: 100%; height: 100%; overflow: visible; }
  line { stroke: #e5e9e6; stroke-dasharray: 3 4; } text { fill: #6b776f; font-size: 10px; font-variant-numeric: tabular-nums; }
  polygon { fill: currentColor; opacity: .07; } polyline { stroke: currentColor; stroke-width: 2; fill: none; vector-effect: non-scaling-stroke; }
  .empty-chart { position: absolute; inset: 45% 0 auto; text-align: center; font-size: 12px; color: #718078; }
</style>
