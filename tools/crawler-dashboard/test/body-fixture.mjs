export function bodyStats() {
  return {
    bodies_started: 120, bodies_admitted: 110, bodies_completed: 100, body_failures: 20,
    rejected_status: 2, rejected_mime: 3, rejected_content_length: 1, rejected_encoding: 4,
    wire_limit_exceeded: 2, decoded_limit_exceeded: 1, stream_failures: 1, allocation_failures: 1,
    idle_timeouts: 2, total_timeouts: 2, budget_timeouts: 1, budget_waits: 12, budget_wait_ms: 340,
    wire_bytes_received: 9437184, decoded_bytes_produced: 8388608,
    active_bodies: 1, reserved_bytes: 20971520, peak_active_bodies: 1, peak_reserved_bytes: 20971520,
    max_inflight_bodies: 4, max_reserved_bytes: 25165824,
  };
}

export function bodyReport() {
  const metric = (value) => ({ median: value, mad: 0, min: value, max: value });
  const scenarios = [
    { scenario: "valid_html", expected: "accepted", written: 4096, received: 4096, decoded: 4096 },
    { scenario: "wrong_mime", expected: "rejected_mime", written: 32768, received: 0, decoded: 0 },
    { scenario: "chunked_limit", expected: "wire_limit_exceeded", written: 4194304, received: 2101248, decoded: 0 },
  ];
  return {
    schema_version: 1, kind: "crawler_body_benchmark", generated_at: new Date().toISOString(),
    configuration: { acceptance_profile: "c05", repetitions: 3, profile: {
      wire_limit_bytes: 2097152, decoded_limit_bytes: 6291456, budget_bytes: 25165824,
      rss_limit_bytes: 268435456, workers: 8,
    } },
    summaries: scenarios.map((item) => ({
      scenario: item.scenario, expected_outcome: item.expected, passed: true, repetitions: 3,
      metrics: {
        duration_ms: metric(30), server_body_bytes_written: metric(item.written),
        body_bytes_received: metric(item.received), body_bytes_decoded: metric(item.decoded),
        peak_reserved_bytes: metric(20971520), rss_peak_bytes: metric(33554432), fd_peak: metric(12),
      },
    })),
    runs: scenarios.flatMap((item) => [1, 2, 3].map((repetition) => ({
      scenario: item.scenario, repetition,
      outcomes: [{ path: `/${item.scenario}`, cause: item.expected, decoded_bytes: item.decoded }],
      body_stats: { ...bodyStats(), wire_bytes_received: item.received, decoded_bytes_produced: item.decoded },
      server: { requests: 1, body_bytes_planned: item.written, body_bytes_written: item.written },
      verification: { passed: true, checks: { expected_outcome: true } },
    }))),
    acceptance: { passed: true, checks: { all_runs_verified: true } },
    code: { commit: "test-c05", source_sha256: "test-source" }, binary: { sha256: "test-binary" },
    build: { profile: "test" }, hardware: { cpu_count: 8 },
  };
}
