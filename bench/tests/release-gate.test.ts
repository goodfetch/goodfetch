// Integration test (spec #152, issue 176): runs a real `bench run` against
// the commit's Proving ground, feeds the resulting cells into the release-gate
// evaluator across both Products, checks correctness blocking, Δ regression
// with/without explanation, competitor rows + behind-competitor list, patch
// correctness-only mode, Evaluation blocking, and Live check reporting.
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import {
  evaluateReleaseGate,
  loadReleaseGateClaims,
  type FamilyBenchmarkRow,
} from "../src/release-gate.ts";
import { bench, written } from "./bench.ts";

describe(
  "release-gate decision",
  {
    tags: [
      "story:2-58",
      "story:2-59",
      "story:2-60",
      "story:2-61",
      "story:2-62",
      "story:2-63",
      "story:2-86",
      "story:2-87",
    ],
  },
  () => {
    test.override("provingGroundSeed", 152);

    test("evaluates release-gate pass, block, Live check and behind-competitor rows from a Proving ground run", async ({
      provingGround,
    }) => {
      const ran = await bench("decisions.json", provingGround.address);
      expect(ran.stderr).toBe("");
      const { record } = await written(ran.out);
      const claimsFile = loadReleaseGateClaims();

      const fastCell = record.cells.find((c) => c.adapter === "fast");
      if (fastCell === undefined) {
        throw new Error("expected fast cell in decisions.json run");
      }

      const candidateRow: FamilyBenchmarkRow = {
        family: "extraction",
        product: "collie",
        adapter: fastCell.adapter,
        is_competitor: false,
        gate_passed: fastCell.gate === "passed",
        verified: fastCell.counts.verified,
        false_success: fastCell.counts.false_success,
        failed: fastCell.counts.failed,
        incomplete: fastCell.counts.incomplete,
        timeout: fastCell.counts.timeout,
        crashed: fastCell.counts.crashed,
        primary_metric_value: fastCell.metrics.task_p50_ms_verified.value,
      };

      const passed = evaluateReleaseGate({
        mode: "release",
        claims: claimsFile.claims,
        candidate_rows: [candidateRow],
        previous_release_rows: [candidateRow],
        behind_competitors: [
          {
            product: "collie",
            family: "extraction",
            competitor: "crawlee-js",
            metric: "task_p50_ms_verified",
            gap: "+4.2%",
            measured_cause: "extra schema validation pass",
            next_cut: "reuse compiled validator across pages",
          },
        ],
        host: {
          host_class: record.host.class_granted,
          host_label: "local-controlled test host",
          refused_by_host_gates: false,
        },
      });

      expect(passed.decision).toBe("pass");
      expect(passed.live_check.tier).toBe("live");
      expect(passed.live_check.status).toBe("pass");
      expect(passed.behind_competitors).toHaveLength(1);

      // Host gate refusal yields `inconclusive` Live check, never `pass`.
      const refused = evaluateReleaseGate({
        mode: "release",
        claims: claimsFile.claims,
        candidate_rows: [candidateRow],
        previous_release_rows: [candidateRow],
        host: {
          host_class: "local-controlled",
          host_label: "busy host",
          refused_by_host_gates: true,
          refusal_reason: "load_per_cpu_1m exceeded limit",
        },
      });
      expect(refused.live_check.status).toBe("inconclusive");
    });
  },
);
