// The benchmark is a separate document, never a crawler status snapshot.
const record = (value) =>
  value !== null && typeof value === "object" && !Array.isArray(value);
const finite = (value) => typeof value === "number" && Number.isFinite(value);
const count = (value) => Number.isSafeInteger(value) && value >= 0;
function benchmarkRun(run) {
  if (!record(run)) return false;
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

export function isBenchmarkReport(report) {
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
        key === "stages_ms"
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
          typeof comparison.saved_pages_equal === "boolean",
      ))
  )
    return false;
  return true;
}
