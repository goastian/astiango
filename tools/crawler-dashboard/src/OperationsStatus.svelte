<script lang="ts">
  import { onMount } from "svelte";
  import { getI18n } from "./i18n";
  const { t, formatLocale } = getI18n();
  type Measurement = { elapsed_ms: number; cpu_ms: number; peak_rss_bytes: number };
  type Evidence = {
    report: { updated_at: string; reader?: { scans: Record<string, { median: Measurement }>; improvements?: Record<string, number> };
      postgres?: { passed: boolean; no_quorum_no_ack: boolean; replicated_payload_bytes: number; discovery_outbox_restart?: boolean; manual_failover?: { manual_rto_seconds: number } };
      worker_drain?: { passed: boolean };
      artifact?: { binary_bytes: number; dependency_isolation_complete: boolean; reproducible_double_build_verified: boolean };
      backup?: { passed: boolean; verified_files: number; restored_frontier_commits: number };
      resources?: { effective_cpus: number; peak_rss_bytes: number; cpu_bound_method: string; cgroup_cpu_memory_controllers_available: boolean };
      worker_scale?: { scenarios: Record<string, { median_pages_per_second: number; repetitions: { max_worker_rss_bytes: number }[] }> };
      gates: Record<string, string> };
    soak?: { observed_seconds: number; gate_24h: string; gate_7d: string; samples: number; failures: unknown[] };
  };
  let evidence = $state<Evidence | null>(null);
  let available = $state(true);
  const number = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 2 }));
  onMount(() => {
    const abort = new AbortController();
    const refresh = async () => {
      try {
        const response = await fetch("/api/operations", { signal: abort.signal });
        if (!response.ok) { available = false; return; }
        evidence = await response.json(); available = true;
      } catch { if (!abort.signal.aborted) available = false; }
    };
    void refresh(); const timer = setInterval(refresh, 60000);
    return () => { abort.abort(); clearInterval(timer); };
  });
</script>

<section class="panel operations-panel" aria-label={$t("Operational verification")}>
  <div class="heading"><h2>{$t("Operational verification")}</h2>
    {#if evidence}<a href="/api/operations" target="_blank" rel="noreferrer">{$t("Descargar informe")}</a>{/if}
  </div>
  <p>{$t("Controlled local fixtures. These results are separate from production crawl counters.")}</p>
  {#if !available}<p>{$t("Operational verification is unavailable.")}</p>{/if}
  {#if evidence?.report.reader}
    <h3>{$t("Streaming archive reader")}</h3>
    <table><thead><tr><th>{$t("Measurement")}</th><th>{$t("Previous reader")}</th><th>{$t("Streaming reader")}</th></tr></thead>
      <tbody>{#each [["Elapsed time", "elapsed_ms", "ms"], ["CPU time", "cpu_ms", "ms"], ["Peak resident memory", "peak_rss_bytes", "MiB"]] as [label, key, unit]}
        <tr><th>{$t(label)}</th>{#each ["legacy", "stream"] as mode}
          <td>{number.format(evidence.report.reader.scans[mode].median[key as keyof Measurement] / (unit === "MiB" ? 1048576 : 1))} {unit}</td>
        {/each}</tr>
      {/each}</tbody>
    </table>
    <p>{$t("Medians of three runs on the same corpus. This is not a global search engine speedup.")}</p>
  {/if}
  {#if evidence?.report.postgres}
    <h3>{$t("Replicated frontier laboratory")}</h3>
    <dl><div><dt>{$t("Replicated capture bytes")}</dt><dd>{number.format(evidence.report.postgres.replicated_payload_bytes)}</dd></div>
      <div><dt>{$t("Manual recovery time")}</dt><dd>{evidence.report.postgres.manual_failover ? number.format(evidence.report.postgres.manual_failover.manual_rto_seconds) + " s" : "—"}</dd></div>
      <div><dt>{$t("No quorum, no acknowledgment")}</dt><dd>{$t(evidence.report.postgres.no_quorum_no_ack ? "Verified" : "Pending verification")}</dd></div>
      <div><dt>{$t("Descubrimiento")}</dt><dd>{$t(evidence.report.postgres.discovery_outbox_restart ? "Verified" : "Pending verification")}</dd></div>
      {#if evidence.report.worker_drain}<div><dt>{$t("Graceful worker shutdown")}</dt><dd>{$t(evidence.report.worker_drain.passed ? "Verified" : "Pending verification")}</dd></div>{/if}
    </dl>
    <p>{$t("Same-host laboratory. Automatic failover remains pending.")}</p>
  {/if}
  {#if evidence?.report.worker_scale}
    <h3>{$t("Native worker scaling")}</h3>
    <div class="table-scroll"><table><thead><tr><th>{$t("Workers")}</th><th>{$t("Pages per second")}</th><th>{$t("Peak resident memory")}</th></tr></thead>
      <tbody>{#each ["1", "2", "4"] as workers}
        {@const scenario = evidence.report.worker_scale.scenarios[workers]}
        <tr><th>{workers}</th><td>{number.format(scenario.median_pages_per_second)}</td><td>{number.format(Math.max(...scenario.repetitions.map(run => run.max_worker_rss_bytes)) / 1048576)} MiB</td></tr>
      {/each}</tbody>
    </table></div>
    <p>{$t("Medians of three runs on the same corpus. This is not a global search engine speedup.")}</p>
  {/if}
  {#if evidence?.report.artifact}
    <h3>{$t("Crawler artifact")}</h3>
    <dl><div><dt>{$t("Artifact size")}</dt><dd>{number.format(evidence.report.artifact.binary_bytes / 1048576)} MiB</dd></div>
      <div><dt>{$t("Engine dependencies excluded")}</dt><dd>{$t(evidence.report.artifact.dependency_isolation_complete ? "Verified" : "Pending verification")}</dd></div>
      <div><dt>{$t("Repeated build verified")}</dt><dd>{$t(evidence.report.artifact.reproducible_double_build_verified ? "Verified" : "Pending verification")}</dd></div>
    </dl>
  {/if}
  {#if evidence?.report.backup}
    <h3>{$t("Backup restoration")}</h3>
    <dl><div><dt>{$t("Files verified")}</dt><dd>{number.format(evidence.report.backup.verified_files)}</dd></div>
      <div><dt>{$t("Restored captures")}</dt><dd>{number.format(evidence.report.backup.restored_frontier_commits)}</dd></div>
    </dl>
  {/if}
  {#if evidence?.report.resources && !evidence.report.resources.cgroup_cpu_memory_controllers_available}
    <p>{$t("CPU affinity verified; cgroup CPU and memory quotas unavailable on this host.")}</p>
  {/if}
  {#if evidence?.soak}
    <h3>{$t("Continuous operation")}</h3>
    <dl><div><dt>{$t("Monitored hours")}</dt><dd>{number.format(evidence.soak.observed_seconds / 3600)}</dd></div>
      <div><dt>24 h</dt><dd>{$t(evidence.soak.gate_24h === "passed" ? "Verified" : "Pending verification")}</dd></div>
      <div><dt>7 d</dt><dd>{$t(evidence.soak.gate_7d === "passed" ? "Verified" : "Pending verification")}</dd></div>
    </dl>
  {/if}
</section>

<style>
  .operations-panel { padding: 22px 24px; } .heading { display: flex; align-items: center; justify-content: space-between; gap: 16px; }
  h2 { margin: 0; font-size: 14px; } h3 { margin: 24px 0 12px; font-size: 13px; }
  p, dt, a { font-size: 12px; color: #687381; line-height: 1.7; } table { width: 100%; border-collapse: collapse; font-size: 13px; }
  th, td { text-align: left; padding: 10px 8px; border-bottom: 1px solid #e7ebf0; } td { font-variant-numeric: tabular-nums; }
  .table-scroll { max-width: 100%; overflow-x: auto; }
  dl { display: flex; flex-wrap: wrap; gap: 24px; } dd { margin: 6px 0 0; font-size: 18px; }
</style>
