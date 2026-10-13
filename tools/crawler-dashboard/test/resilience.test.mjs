import assert from "node:assert/strict";
import { test } from "node:test";
import { isBenchmarkReport } from "../benchmark-report.mjs";

function report() {
  const d = { median: 10, min: 9, max: 11, mad: 1 };
  return { schema_version:1,kind:"crawler_resilience_benchmark",generated_at:"2026-10-12T12:00:00Z",
    code:{commit:"a".repeat(40),source_sha256:"b".repeat(64)},
    configuration:{repetitions:3,pages:20,planner_cache_bytes:8388608},
    comparisons:{body_bytes:40,cpu_ms:-2,duration_ms:5},acceptance:{passed:true},
    recovery:[1,2,3].map(repetition=>({repetition,urls:100000,replayed_from:128})),
    scenarios:["recrawl","planner","discovery","traps","diversity"].map(name=>({name,repetitions:3,
      metrics:{duration_ms:d,cpu_ms:d,rss_after_bytes:d},runs:[1,2,3].map(repetition=>({repetition}))})) };
}
test("C14–C20 reports require three finite repetitions and reproducible code",()=>{
  assert.equal(isBenchmarkReport(report()),true);
  for (const mutate of [r=>r.scenarios[0].runs.pop(),r=>r.scenarios[0].metrics.cpu_ms.median=NaN,
    r=>r.configuration.repetitions=1,r=>r.code.source_sha256="unknown",r=>r.recovery.pop(),
    r=>r.comparisons.duration_ms=Infinity]) {
    const value=report();mutate(value);assert.equal(isBenchmarkReport(value),false);
  }
});
