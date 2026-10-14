<script lang="ts">
  import { onMount } from "svelte";
  import { getI18n } from "./i18n";
  const { t, formatLocale } = getI18n();
  type Measurement = { elapsed_ms: number; cpu_ms: number; peak_rss_bytes: number };
  type Evidence = {
    report: { updated_at: string; reader?: { scans: Record<string, { median: Measurement }>; improvements?: Record<string, number> };
      postgres?: { passed: boolean; no_quorum_no_ack: boolean; replicated_payload_bytes: number; discovery_outbox_restart?: boolean; manual_failover?: { manual_rto_seconds: number } };
      distributed_complete?: { passed: boolean; robots_downloads_after_restart: number; fetch_drain_seconds: number; commit_drain_seconds: number };
      distributed_policy?: { passed: boolean; shared_crawl_delay_seconds: number; hot_origin_yields: boolean };
      redundant_control?: { passed: boolean; controllers: number; controller_loss_rto_seconds: number; no_dcs_quorum_no_primary: boolean; local_data_directory_loss_rebuilt: boolean };
      specialized_production?: { passed: boolean; added_text_bytes: number; cohort: { url: string }[]; results: { state: string }[]; index_consumer: { indexed: number; duplicates: number; noindex: number }[] | null };
      worker_drain?: { passed: boolean };
      quick_checks?: { passed: boolean; elapsed_seconds: number };
      local_failover?: { passed: boolean; rto_seconds: number; one_partition_rebalanced: boolean };
      extreme_html_memory?: { passed: boolean; peak_rss_bytes: number };
      replication_recovery?: { passed: boolean; recovery_http_requests: number };
      pdf_extraction?: { passed: boolean; median_elapsed_ms: number; production_enabled: boolean };
      artifact?: { binary_bytes: number; dependency_isolation_complete: boolean; reproducible_double_build_verified: boolean };
      backup?: { passed: boolean; verified_files: number; restored_frontier_commits: number };
      resources?: { effective_cpus: number; peak_rss_bytes: number; cpu_bound_method: string; cgroup_cpu_memory_controllers_available: boolean };
      worker_scale?: { scenarios: Record<string, { median_pages_per_second: number; repetitions: { max_worker_rss_bytes: number }[] }> };
      gates: Record<string, string> };
    soak?: { observed_seconds: number; gate_24h: string; gate_7d: string; samples: number; failures: unknown[]; status?: string };
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
    <p>{$t("Same-host laboratory. Independent disk failure remains unverified.")}</p>
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
  {#if evidence?.report.quick_checks}
    <h3>{$t("Short local integration checks")}</h3>
    <dl>
      <div><dt>{$t("Verificación")}</dt><dd>{$t(evidence.report.quick_checks.passed ? "Verified" : "Pending verification")} · {number.format(evidence.report.quick_checks.elapsed_seconds)} s</dd></div>
      {#if evidence.report.extreme_html_memory}<div><dt>{$t("Simultaneous HTML memory")}</dt><dd>{number.format(evidence.report.extreme_html_memory.peak_rss_bytes / 1048576)} / 512 MiB</dd></div>{/if}
      {#if evidence.report.local_failover}<div><dt>{$t("Automatic local recovery")}</dt><dd>{number.format(evidence.report.local_failover.rto_seconds)} s</dd></div>{/if}
      {#if evidence.report.replication_recovery}<div><dt>{$t("Recovery HTTP requests")}</dt><dd>{number.format(evidence.report.replication_recovery.recovery_http_requests)}</dd></div>{/if}
      {#if evidence.report.pdf_extraction}<div><dt>{$t("Isolated PDF extraction")}</dt><dd>{number.format(evidence.report.pdf_extraction.median_elapsed_ms)} ms</dd></div>{/if}
    </dl>
  {/if}
  {#if evidence?.report.distributed_complete}
    <h3>{$t("Distributed acquisition")}</h3>
    <dl>
      <div><dt>{$t("Verificación")}</dt><dd>{$t(evidence.report.distributed_complete.passed ? "Verified" : "Pending verification")}</dd></div>
      <div><dt>{$t("Robots downloads after restart")}</dt><dd>{number.format(evidence.report.distributed_complete.robots_downloads_after_restart)}</dd></div>
      <div><dt>{$t("Fetch / commit drain")}</dt><dd>{number.format(evidence.report.distributed_complete.fetch_drain_seconds)} / {number.format(evidence.report.distributed_complete.commit_drain_seconds)} s</dd></div>
      {#if evidence.report.distributed_policy}<div><dt>{$t("Shared crawl delay")}</dt><dd>{number.format(evidence.report.distributed_policy.shared_crawl_delay_seconds)} s</dd></div>{/if}
    </dl>
  {/if}
  {#if evidence?.report.redundant_control}
    <h3>{$t("Redundant control plane")}</h3>
    <dl>
      <div><dt>{$t("Controllers")}</dt><dd>{number.format(evidence.report.redundant_control.controllers)}</dd></div>
      <div><dt>{$t("Controller recovery")}</dt><dd>{number.format(evidence.report.redundant_control.controller_loss_rto_seconds)} s</dd></div>
      <div><dt>{$t("No quorum, no acknowledgment")}</dt><dd>{$t(evidence.report.redundant_control.no_dcs_quorum_no_primary ? "Verified" : "Pending verification")}</dd></div>
      <div><dt>{$t("Lost data directory recovery")}</dt><dd>{$t(evidence.report.redundant_control.local_data_directory_loss_rebuilt ? "Verified" : "Pending verification")}</dd></div>
    </dl>
    <p>{$t("Same-host laboratory. Independent disk failure remains unverified.")}</p>
  {/if}
  {#if evidence?.report.specialized_production}
    <h3>{$t("Specialized extraction")}</h3>
    <dl>
      <div><dt>{$t("Archived real-web cohort")}</dt><dd>{number.format(evidence.report.specialized_production.cohort.length)}</dd></div>
      <div><dt>{$t("Additional extracted text")}</dt><dd>{number.format(evidence.report.specialized_production.added_text_bytes)} B</dd></div>
      <div><dt>{$t("Completed / quota failures")}</dt><dd>{number.format(evidence.report.specialized_production.results.filter(row => row.state === "committed").length)} / {number.format(evidence.report.specialized_production.results.filter(row => row.state === "failed").length)}</dd></div>
      {#if evidence.report.specialized_production.index_consumer}<div><dt>{$t("Derived documents indexed")}</dt><dd>{number.format(evidence.report.specialized_production.index_consumer[0].indexed)}</dd></div>{/if}
    </dl>
    <p>{$t("Isolated offline JavaScript. Network subresources stay blocked; global coverage is not estimated.")}</p>
  {/if}
  {#if evidence?.soak}
    <h3>{$t("Continuous operation")}</h3>
    <dl><div><dt>{$t("Monitored hours")}</dt><dd>{number.format(evidence.soak.observed_seconds / 3600)}</dd></div>
      <div><dt>24 h</dt><dd>{$t(evidence.soak.gate_24h === "passed" ? "Verified" : evidence.soak.status === "stopped_by_user" ? "Optional observation" : "Pending verification")}</dd></div>
      <div><dt>7 d</dt><dd>{$t(evidence.soak.gate_7d === "passed" ? "Verified" : evidence.soak.status === "stopped_by_user" ? "Optional observation" : "Pending verification")}</dd></div>
    </dl>
    {#if evidence.soak.status === "stopped_by_user"}<p>{$t("Prolonged monitoring stopped. You can suspend the computer.")}</p>{/if}
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
