<script lang="ts">
  import { getI18n } from "./i18n";
  import type { ResourceStats } from "./types";
  const { t, formatLocale } = getI18n();
  let { stats, paused = false, readError = false }: { stats?: ResourceStats; paused?: boolean; readError?: boolean } = $props();
  const number = $derived(new Intl.NumberFormat($formatLocale, { maximumFractionDigits: 1 }));
  const reasons: Record<string, string> = {
    recovering: "Recovering admission gradually", healthy: "Resources available", manual: "Manual admission", spool_full: "Spool capacity reached",
    disk_reserve: "Disk reserve reached", inode_reserve: "Inode reserve reached", fd_limit: "File descriptor limit reached",
    memory_limit: "Memory limit reached", memory_pressure: "Memory pressure", network_budget: "Daily network budget reached",
  };
  const metrics = $derived(stats ? [
    ["Effective CPUs", stats.effective_cpus, ""], ["Process CPU usage", stats.process_cpu_percent, "%"],
    ["Resident memory", stats.rss_bytes / 1048576, "MiB"], ["Memory budget", stats.memory_limit_bytes / 1048576, "MiB"],
    ["Free disk", stats.disk_free_bytes / 1073741824, "GiB"], ["Free inodes", stats.free_inodes, ""],
    ["Open file descriptors", stats.open_fds, ""], ["Workers admitted", stats.workers_allowed, ""],
    ["Worker ceiling", stats.workers_max, ""], ["Admission adjustments", stats.adjustments, ""],
    ["Process disk reads", stats.process_read_bytes / 1048576, "MiB"], ["Process disk writes", stats.process_written_bytes / 1048576, "MiB"],
    ["Received today", stats.network_bytes_today / 1048576, "MiB"],
  ] as [string, number, string][] : []);
</script>

<section class="panel resource-panel" aria-label={$t("Resources and admission")}>
  <div class="resource-heading"><h2>{$t("Resources and admission")}</h2>
    <span class:blocked={stats?.automatic && stats.workers_allowed === 0}>{stats ? $t(reasons[stats.reason] ?? stats.reason) : "—"}</span>
  </div>
  {#if stats && stats.sampled_at}
    {#if paused || readError}<p>{$t("Showing the last available resource sample.")}</p>{/if}
    <dl>{#each metrics as [label, value, unit]}<div><dt>{$t(label)}</dt><dd>{number.format(value)} {unit}</dd></div>{/each}</dl>
    <p>{$t("CPU usage is relative to one core. Resident memory is measured; reserved buffers and virtual stacks are separate.")}</p>
  {:else}
    <p>{$t("Resource probes have not been published by this crawler yet.")}</p>
  {/if}
</section>

<style>
  .resource-panel { padding: 22px 24px; }
  .resource-heading { display: flex; gap: 16px; justify-content: space-between; align-items: center; flex-wrap: wrap; }
  h2 { font-size: 14px; margin: 0; } span, p, dt { font-size: 12px; color: #687381; }
  .blocked { color: #a34b12; }
  dl { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 20px; margin: 24px 0; }
  dd { margin: 6px 0 0; font-size: 18px; font-variant-numeric: tabular-nums; }
  p { line-height: 1.7; margin-bottom: 0; }
  @media (max-width: 900px) { dl { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
</style>
