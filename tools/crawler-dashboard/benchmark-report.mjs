// The benchmark is a separate document, never a crawler status snapshot.
const record = (value) =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const finite = (value) => typeof value === "number" && Number.isFinite(value);
const count = (value) => Number.isSafeInteger(value) && value >= 0;
const observerMode = (value) => ["legacy_sync", "async", "off"].includes(value);
function benchmarkRun(run) {
  if (!record(run)) return false;
  if (run.observer_mode !== undefined && !observerMode(run.observer_mode)) return false;
  if (
    run.repetition !== undefined &&
    (!count(run.repetition) || run.repetition === 0)
  )
    return false;
  if (
    ["request_failures", "saved_pages", "client_requests"].some(
      (key) => run[key] !== undefined && !count(run[key]),
    )
  )
    return false;
  if (
    run.errors !== undefined &&
    (!record(run.errors) || !Object.values(run.errors).every(count))
  )
    return false;
  if (
    run.requests_match !== undefined &&
    typeof run.requests_match !== "boolean"
  )
    return false;
  if (
    run.server !== undefined &&
    (!record(run.server) ||
      (run.server.requests !== undefined && !count(run.server.requests)))
  )
    return false;
  return true;
}
function distribution(value) {
  return (
    record(value) &&
    ["median", "min", "max", "mad"].every(
      (key) => finite(value[key]) && value[key] >= 0,
    ) &&
    value.min <= value.median &&
    value.median <= value.max
  );
}
function signedDistribution(value) {
  return record(value) && ["median", "min", "max", "mad"].every((key) => finite(value[key])) &&
    value.mad >= 0 && value.min <= value.median && value.median <= value.max;
}

export function isBenchmarkReport(report) {
  if (record(report) && report.kind === "crawler_extraction_benchmark")
    return isExtractionBenchmarkReport(report);
  if (record(report) && report.kind === "crawler_body_benchmark")
    return isBodyBenchmarkReport(report);
  if (record(report) && report.kind === "crawler_transport_benchmark")
    return isTransportBenchmarkReport(report);
  if (
    !record(report) ||
    report.schema_version !== 1 ||
    report.kind !== "crawler-capacity-benchmark"
  )
    return false;
  if (
    typeof report.generated_at !== "string" ||
    !Number.isFinite(Date.parse(report.generated_at))
  )
    return false;
  if (
    !record(report.configuration) ||
    !record(report.acceptance) ||
    typeof report.acceptance.passed !== "boolean"
  )
    return false;
  if (!Array.isArray(report.scenarios) || report.scenarios.length === 0)
    return false;
  if (
    !report.scenarios.every((scenario) => {
      if (
        !record(scenario) ||
        typeof scenario.name !== "string" ||
        !scenario.name
      )
        return false;
      if (!Number.isSafeInteger(scenario.origins) || scenario.origins <= 0)
        return false;
      if (
        typeof scenario.observer !== "boolean" &&
        !["on", "off", "enabled", "disabled"].includes(scenario.observer)
      )
        return false;
      if (scenario.observer_mode !== undefined && !observerMode(scenario.observer_mode)) return false;
      if (
        !Number.isSafeInteger(scenario.repetitions) ||
        scenario.repetitions <= 0 ||
        typeof scenario.requests_match !== "boolean"
      )
        return false;
      if (
        !record(scenario.metrics) ||
        !distribution(scenario.metrics.duration_ms)
      )
        return false;
      if (
        scenario.runs !== undefined &&
        (!Array.isArray(scenario.runs) || !scenario.runs.every(benchmarkRun))
      )
        return false;
      return Object.entries(scenario.metrics).every(([key, value]) =>
        key === "stages_ms" || key === "observer_stats"
          ? record(value) && Object.values(value).every(distribution)
          : distribution(value),
      );
    })
  )
    return false;
  if (
    report.comparisons !== undefined &&
    (!Array.isArray(report.comparisons) ||
      !report.comparisons.every(
        (comparison) =>
          record(comparison) &&
          typeof comparison.name === "string" &&
          Number.isSafeInteger(comparison.origins) &&
          (comparison.duration_overhead_pct == null ||
            finite(comparison.duration_overhead_pct)) &&
          typeof comparison.requests_equal === "boolean" &&
          typeof comparison.saved_pages_equal === "boolean" &&
          (comparison.behavior_equivalent === undefined || typeof comparison.behavior_equivalent === "boolean") &&
          ["write_calls_reduction_pct", "writes_per_update_reduction_pct"].every((key) => comparison[key] === undefined || signedDistribution(comparison[key])),
      ))
  )
    return false;
  return true;
}

function transportIdentity(value) {
  return record(value) && ["http", "https"].includes(value.scheme) &&
    Number.isSafeInteger(value.origins) && value.origins > 0;
}

function isTransportBenchmarkReport(report) {
  if (report.schema_version !== 1 || typeof report.generated_at !== "string" ||
      !Number.isFinite(Date.parse(report.generated_at)) || !record(report.configuration) ||
      report.configuration.acceptance_profile !== "c04" || !record(report.acceptance) ||
      typeof report.acceptance.passed !== "boolean") return false;
  if (!Array.isArray(report.summaries) || report.summaries.length === 0 ||
      !report.summaries.every((row) => transportIdentity(row) &&
        ["legacy", "pooled"].includes(row.mode) && count(row.repetitions) && row.repetitions > 0 &&
        record(row.metrics) && distribution(row.metrics.connections_per_page) &&
        ["duration_ms", "requests", "pages", "connections"].every((key) => distribution(row.metrics[key])) &&
        Object.values(row.metrics).every(distribution))) return false;
  const identities = report.summaries.map((row) => `${row.scheme}:${row.origins}:${row.mode}`);
  if (new Set(identities).size !== identities.length) return false;
  if (!Array.isArray(report.runs) || !report.runs.every((run) => transportIdentity(run) &&
      ["legacy", "pooled"].includes(run.mode) && count(run.repetition) && run.repetition > 0 &&
      count(run.pages) && count(run.requests) && record(run.server) &&
      count(run.server.connections) && count(run.server.requests) &&
      ["fd_baseline", "fd_peak", "fd_final"].every((key) => run[key] === undefined || count(run[key])))) return false;
  return report.comparisons === undefined || (Array.isArray(report.comparisons) &&
    report.comparisons.every((row) => transportIdentity(row) && typeof row.equivalent === "boolean" &&
      signedDistribution(row.connections_per_page_reduction_pct) &&
      (row.dns_queries_per_page_reduction_pct === undefined || signedDistribution(row.dns_queries_per_page_reduction_pct))));
}

function isBodyBenchmarkReport(report) {
  if (report.schema_version !== 1 || typeof report.generated_at !== "string" ||
      !Number.isFinite(Date.parse(report.generated_at)) || !record(report.configuration) ||
      report.configuration.acceptance_profile !== "c05" || !record(report.configuration.profile) ||
      !record(report.acceptance) || typeof report.acceptance.passed !== "boolean") return false;
  if (!Array.isArray(report.summaries) || report.summaries.length === 0 ||
      !report.summaries.every((row) => record(row) && typeof row.scenario === "string" && row.scenario &&
        typeof row.expected_outcome === "string" && typeof row.passed === "boolean" &&
        count(row.repetitions) && row.repetitions > 0 && record(row.metrics) &&
        ["duration_ms", "server_body_bytes_written", "body_bytes_received", "body_bytes_decoded", "peak_reserved_bytes", "rss_peak_bytes"].every((key) => distribution(row.metrics[key])) &&
        Object.values(row.metrics).every(distribution))) return false;
  if (new Set(report.summaries.map((row) => row.scenario)).size !== report.summaries.length) return false;
  return Array.isArray(report.runs) && report.runs.every((run) => record(run) &&
    typeof run.scenario === "string" && count(run.repetition) && run.repetition > 0 &&
    record(run.body_stats) && record(run.server) && record(run.verification) && typeof run.verification.passed === "boolean" &&
    ["wire_bytes_received", "decoded_bytes_produced"].every((key) => count(run.body_stats[key])) &&
    ["requests", "body_bytes_planned", "body_bytes_written"].every((key) => count(run.server[key])) &&
    Array.isArray(run.outcomes) && run.outcomes.every((outcome) => record(outcome) &&
      typeof outcome.path === "string" && typeof outcome.cause === "string" && count(outcome.decoded_bytes)));
}

function isExtractionBenchmarkReport(report) {
  const modes = (mode) => ["legacy", "shared"].includes(mode);
  if (report.schema_version !== 1 || !Number.isFinite(Date.parse(report.generated_at)) ||
      !record(report.configuration) || report.configuration.acceptance_profile !== "c06" ||
      !record(report.acceptance) || typeof report.acceptance.passed !== "boolean" ||
      !signedDistribution(report.corpus_cpu_reduction_pct)) return false;
  const metricNames = ["cpu_ms_per_page", "wall_ms_per_page", "allocated_bytes_per_page", "allocation_calls_per_page", "dom_parses_per_page"];
  if (!Array.isArray(report.summaries) || !report.summaries.length || !report.summaries.every((row) =>
    record(row) && typeof row.scenario === "string" && modes(row.mode) && count(row.repetitions) && row.repetitions > 0 &&
    record(row.metrics) && metricNames.every((key) => distribution(row.metrics[key])))) return false;
  if (new Set(report.summaries.map((row) => `${row.scenario}:${row.mode}`)).size !== report.summaries.length) return false;
  if (!Array.isArray(report.comparisons) || !report.comparisons.every((row) => record(row) && typeof row.scenario === "string" &&
    typeof row.equivalent === "boolean" && signedDistribution(row.cpu_reduction_pct) && signedDistribution(row.allocated_bytes_reduction_pct))) return false;
  return Array.isArray(report.runs) && report.runs.every((run) => record(run) && typeof run.scenario === "string" && modes(run.mode) &&
    count(run.repetition) && run.repetition > 0 && metricNames.every((key) => finite(run[key]) && run[key] >= 0) &&
    record(run.extraction) && count(run.extraction.anchors_visited) && typeof run.extraction.base_url === "string" &&
    ["no_index", "no_follow", "traversal_stopped"].every((key) => typeof run.extraction[key] === "boolean") &&
    ["discovery_urls", "internal_urls"].every((key) => Array.isArray(run.extraction[key]) && run.extraction[key].length <= 512 && run.extraction[key].every((url) => typeof url === "string")));
}
