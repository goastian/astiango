<script lang="ts">
  import { onMount } from "svelte";
  import { getI18n } from "./i18n";
  const { t, formatLocale } = getI18n();
  let { runId, paused }: { runId?: string; paused: boolean } = $props();
  type Sample = { run_id: string; at: string; pages: number; http_attempts: number | null; wire_bytes: number | null; disk_written_bytes: number | null };
  let samples = $state<Sample[]>([]);
  let failure = $state(false);
  const number = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 2 }));
  const current = $derived(samples.filter(sample => sample.run_id === runId));
  const latest = $derived(current.at(-1));
  const previous = $derived(current.at(-2));
  const interval = $derived(latest && previous ? (Date.parse(latest.at) - Date.parse(previous.at)) / 1000 : 0);
  function rate(key: keyof Pick<Sample,"pages"|"http_attempts"|"wire_bytes"|"disk_written_bytes">, divisor=1) {
    if (!latest || !previous || interval<=0 || latest[key]===null || previous[key]===null || latest[key]!<previous[key]!) return "—";
    return number.format((latest[key]! - previous[key]!) / interval / divisor);
  }
  const cost = $derived.by(() => {
    const first=current[0];
    if (!first || !latest || first.wire_bytes===null || latest.wire_bytes===null || latest.pages<=first.pages || latest.wire_bytes<first.wire_bytes) return "—";
    return number.format((latest.wire_bytes-first.wire_bytes)/(latest.pages-first.pages)*1000/1048576);
  });
  onMount(() => {
    const abort=new AbortController();
    const refresh=async () => {
      if (paused || !runId) return;
      try { const response=await fetch(`/api/series?run_id=${encodeURIComponent(runId)}`,{signal:abort.signal});
        if (!response.ok) throw new Error("series unavailable");
        const value=await response.json(); samples=value.samples; failure=!!value.error;
      } catch { if (!abort.signal.aborted) failure=true; }
    };
    void refresh(); const timer=setInterval(refresh,5000);
    return ()=>{abort.abort();clearInterval(timer);};
  });
</script>

<section class="panel rates-panel" aria-label={$t("Observed throughput")}>
  <h2>{$t("Observed throughput")}</h2>
  <dl><div><dt>{$t("Durable pages per second")}</dt><dd>{rate("pages")}</dd></div>
    <div><dt>{$t("HTTP attempts per second")}</dt><dd>{rate("http_attempts")}</dd></div>
    <div><dt>{$t("Received KiB per second")}</dt><dd>{rate("wire_bytes",1024)}</dd></div>
    <div><dt>{$t("Disk writes KiB per second")}</dt><dd>{rate("disk_written_bytes",1024)}</dd></div>
    <div><dt>{$t("Received MiB per 1,000 durable pages")}</dt><dd>{cost}</dd></div></dl>
  <p>{$t("Rates use consecutive samples from this run. Cost covers the retained observation window.")}</p>
  {#if failure}<p>{$t("Showing the last available resource sample.")}</p>{/if}
</section>

<style>
  .rates-panel {padding:22px 24px;} h2 {margin:0;font-size:14px;} dl {display:grid;grid-template-columns:repeat(5,minmax(0,1fr));gap:20px;margin:24px 0;}
  dt,p {font-size:12px;color:#687381;line-height:1.7;} dd {margin:6px 0 0;font-size:18px;font-variant-numeric:tabular-nums;}
  @media(max-width:1000px){dl{grid-template-columns:repeat(2,minmax(0,1fr));}}
</style>
