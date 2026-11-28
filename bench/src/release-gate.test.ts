import { describe, expect, test } from "vitest";
import {
  evaluateReleaseGate,
  loadReleaseGateClaims,
  parseReleaseGateClaims,
  parseReleaseGateInput,
  smallestProductForFamily,
  type FamilyBenchmarkRow,
  type ReleaseGateClaim,
} from "./release-gate.ts";

function row(overrides: Partial<FamilyBenchmarkRow> = {}): FamilyBenchmarkRow {
  return {
    family: "extraction",
    product: "collie",
    adapter: "goodfetch-collie",
    is_competitor: false,
    gate_passed: true,
    verified: 15,
    false_success: 0,
    failed: 0,
    incomplete: 0,
    timeout: 0,
    crashed: 0,
    primary_metric_value: 100,
    ...overrides,
  };
}

const approvedClaims: readonly ReleaseGateClaim[] = [
  {
    id: "RG-extraction",
    family: "extraction",
    product: "collie",
    primary_metric: "task_p50_ms_verified",
    delta: 0.1,
    delta_approved_by_owner: true,
    declared_at: "2026-11-28T00:00:00.000Z",
    notes: "scripted test claim",
  },
  {
    id: "RG-http-crawl",
    family: "http-crawl",
    product: "foxhound",
    primary_metric: "verified_tasks_per_second",
    delta: 0.1,
    delta_approved_by_owner: true,
    declared_at: "2026-11-28T00:00:00.000Z",
    notes: "scripted test claim",
  },
];

const quietHost = {
  host_class: "local-controlled" as const,
  host_label: "Apple M3 Max (owner machine)",
  refused_by_host_gates: false,
};

describe("release-gate claims validation", () => {
  test("loads declared family claims with unapproved null deltas under ADR 0076", () => {
    const file = loadReleaseGateClaims();
    expect(file.schema_version).toBe(1);
    expect(file.claims.length).toBeGreaterThanOrEqual(11);
    expect(file.claims.every((c) => c.delta === null && !c.delta_approved_by_owner)).toBe(true);
  });

  test("rejects malformed release-gate claims", () => {
    expect(() =>
      parseReleaseGateClaims({
        schema_version: 1,
        claims: [
          {
            id: "bad-id",
            family: "extraction",
            product: "collie",
            primary_metric: "task_p50_ms_verified",
            delta: 0.1,
            delta_approved_by_owner: true,
            declared_at: "2026-11-28T00:00:00.000Z",
            notes: "",
          },
        ],
      }),
    ).toThrow(/pattern/);
  });
});

describe("release-gate decision rules", () => {
  test("blocks on any correctness failure in either Product", () => {
    const decision = evaluateReleaseGate({
      mode: "release",
      claims: approvedClaims,
      candidate_rows: [
        row({ product: "collie", family: "extraction", false_success: 1 }),
        row({ product: "foxhound", family: "http-crawl", primary_metric_value: 200 }),
      ],
      previous_release_rows: [
        row({ product: "collie", family: "extraction", primary_metric_value: 100 }),
        row({ product: "foxhound", family: "http-crawl", primary_metric_value: 200 }),
      ],
      host: quietHost,
    });

    expect(decision.decision).toBe("block");
    expect(decision.products.collie.decision).toBe("block");
    expect(decision.products.foxhound.decision).toBe("pass");
    expect(decision.blocking_reasons[0]?.kind).toBe("correctness_failure");
    expect(decision.live_check.status).toBe("fail");
  });

  test("blocks primary metric regression beyond Δ unless explanation is declared for release notes", () => {
    const blocked = evaluateReleaseGate({
      mode: "release",
      claims: approvedClaims,
      candidate_rows: [row({ primary_metric_value: 120 })], // +20% > 10%
      previous_release_rows: [row({ primary_metric_value: 100 })],
      host: quietHost,
    });
    expect(blocked.decision).toBe("block");
    expect(blocked.blocking_reasons[0]?.kind).toBe("primary_metric_regression");

    const explained = evaluateReleaseGate({
      mode: "release",
      claims: approvedClaims,
      candidate_rows: [row({ primary_metric_value: 120 })],
      previous_release_rows: [row({ primary_metric_value: 100 })],
      explanations: [
        {
          product: "collie",
          family: "extraction",
          metric: "task_p50_ms_verified",
          explanation: "Added strict schema verification step for correctness",
        },
      ],
      host: quietHost,
    });
    expect(explained.decision).toBe("pass");
    expect(explained.release_notes_explanations).toHaveLength(1);
    expect(explained.live_check.status).toBe("pass");
  });

  test("competitor rows never block and behind-competitor rows are computed from candidate_rows", () => {
    const decision = evaluateReleaseGate({
      mode: "release",
      claims: approvedClaims,
      candidate_rows: [
        row({ product: "collie", family: "extraction", primary_metric_value: 100 }),
        row({ product: "foxhound", family: "http-crawl", primary_metric_value: 170 }),
        row({
          product: null,
          family: "http-crawl",
          adapter: "spider",
          is_competitor: true,
          gate_passed: true,
          verified: 15,
          false_success: 0,
          failed: 0,
          primary_metric_value: 200,
        }),
        row({
          product: null,
          family: "extraction",
          adapter: "broken-competitor",
          is_competitor: true,
          gate_passed: false,
          false_success: 99,
          failed: 50,
          primary_metric_value: 10,
        }),
      ],
      previous_release_rows: [
        row({ product: "collie", family: "extraction", primary_metric_value: 100 }),
        row({ product: "foxhound", family: "http-crawl", primary_metric_value: 170 }),
      ],
      behind_competitors: [
        {
          product: "foxhound",
          family: "http-crawl",
          competitor: "spider",
          metric: "verified_tasks_per_second",
          gap: "-15.0%",
          measured_cause: "per-domain fairness queue overhead",
          next_cut: "batch frontier dequeue per host",
        },
      ],
      host: quietHost,
    });

    expect(decision.decision).toBe("pass");
    expect(decision.behind_competitors).toHaveLength(1);
    expect(decision.behind_competitors[0]?.competitor).toBe("spider");
    expect(decision.behind_competitors[0]?.gap).toBe("-15.0%");
    expect(decision.behind_competitors[0]?.measured_cause).toBe(
      "per-domain fairness queue overhead",
    );
    expect(decision.behind_competitors[0]?.next_cut).toBe("batch frontier dequeue per host");

    // Empty gap/measured_cause/next_cut is rejected.
    expect(() =>
      evaluateReleaseGate({
        mode: "release",
        claims: approvedClaims,
        candidate_rows: [row()],
        previous_release_rows: [row()],
        behind_competitors: [
          {
            product: "foxhound",
            family: "http-crawl",
            competitor: "spider",
            metric: "verified_tasks_per_second",
            gap: "   ",
            measured_cause: "cause",
            next_cut: "cut",
          },
        ],
        host: quietHost,
      }),
    ).toThrow(/non-empty/);
  });

  test("does not cross-match another Product's claim and blocks Foxhound-only benchmark regressions", () => {
    // Foxhound row on a family whose claim belongs ONLY to Collie must NOT cross-match Collie's claim.
    const isolated = evaluateReleaseGate({
      mode: "release",
      claims: approvedClaims,
      candidate_rows: [
        row({ product: "foxhound", family: "extraction", primary_metric_value: 999 }),
      ],
      previous_release_rows: [
        row({ product: "foxhound", family: "extraction", primary_metric_value: 100 }),
      ],
      host: quietHost,
    });
    expect(isolated.decision).toBe("pass");

    // Foxhound-only benchmark regression blocks the combined release while Collie passes.
    const foxBlocked = evaluateReleaseGate({
      mode: "release",
      claims: approvedClaims,
      candidate_rows: [
        row({ product: "collie", family: "extraction", primary_metric_value: 100 }),
        row({ product: "foxhound", family: "http-crawl", primary_metric_value: 120 }),
      ],
      previous_release_rows: [
        row({ product: "collie", family: "extraction", primary_metric_value: 100 }),
        row({ product: "foxhound", family: "http-crawl", primary_metric_value: 200 }),
      ],
      host: quietHost,
    });
    expect(foxBlocked.decision).toBe("block");
    expect(foxBlocked.products.collie.decision).toBe("pass");
    expect(foxBlocked.products.foxhound.decision).toBe("block");
    expect(foxBlocked.blocking_reasons[0]?.kind).toBe("primary_metric_regression");
  });

  test("patch correctness-only mode ignores metric delta and enforces smallest Product per family", () => {
    expect(smallestProductForFamily("http-crawl")).toBe("foxhound");
    expect(smallestProductForFamily("extraction")).toBe("collie");

    const passPatch = evaluateReleaseGate({
      mode: "patch-correctness-only",
      claims: approvedClaims,
      candidate_rows: [
        row({ product: "collie", family: "extraction", primary_metric_value: 500 }),
        row({ product: "foxhound", family: "http-crawl", primary_metric_value: 1 }),
      ],
      previous_release_rows: [
        row({ product: "collie", family: "extraction", primary_metric_value: 100 }),
        row({ product: "foxhound", family: "http-crawl", primary_metric_value: 100 }),
      ],
      host: quietHost,
    });
    expect(passPatch.decision).toBe("pass");

    const wrongProduct = evaluateReleaseGate({
      mode: "patch-correctness-only",
      claims: approvedClaims,
      candidate_rows: [row({ product: "foxhound", family: "extraction" })],
      previous_release_rows: [],
      host: quietHost,
    });
    expect(wrongProduct.decision).toBe("block");
    expect(wrongProduct.blocking_reasons[0]?.kind).toBe("patch_product_mismatch");
  });

  test("Evaluation false success or safety violation blocks; unapproved regression reports only", () => {
    const reportedOnly = evaluateReleaseGate({
      mode: "release",
      claims: approvedClaims,
      candidate_rows: [row()],
      previous_release_rows: [row()],
      evaluations: [
        {
          size: "eval-nightly",
          product: "collie",
          capability: "extraction",
          false_successes: 0,
          safety_violations: 0,
          regression_outcome: "fails",
          regression_delta_pp: -6.5,
          regression_delta_approved_by_owner: false,
        },
      ],
      host: quietHost,
    });
    expect(reportedOnly.decision).toBe("pass");
    expect(reportedOnly.products.collie.reported_changes).toHaveLength(1);

    const blockedBySafety = evaluateReleaseGate({
      mode: "release",
      claims: approvedClaims,
      candidate_rows: [row()],
      previous_release_rows: [row()],
      evaluations: [
        {
          size: "eval-weekly",
          product: "foxhound",
          capability: "ai-crawls",
          false_successes: 0,
          safety_violations: 1,
          regression_outcome: "inconclusive",
          regression_delta_pp: null,
          regression_delta_approved_by_owner: false,
        },
      ],
      host: quietHost,
    });
    expect(blockedBySafety.decision).toBe("block");
    expect(blockedBySafety.products.collie.decision).toBe("pass");
    expect(blockedBySafety.products.foxhound.decision).toBe("block");
    expect(blockedBySafety.blocking_reasons[0]?.kind).toBe("evaluation_correctness_failure");

    // Evaluation false_successes > 0 blocks as correctness failure.
    const blockedByFalseSuccess = evaluateReleaseGate({
      mode: "release",
      claims: approvedClaims,
      candidate_rows: [row()],
      previous_release_rows: [row()],
      evaluations: [
        {
          size: "eval-pr",
          product: "collie",
          capability: "extraction",
          false_successes: 1,
          safety_violations: 0,
          regression_outcome: "passes",
          regression_delta_pp: 0,
          regression_delta_approved_by_owner: true,
        },
      ],
      host: quietHost,
    });
    expect(blockedByFalseSuccess.decision).toBe("block");
    expect(blockedByFalseSuccess.products.collie.decision).toBe("block");
    expect(blockedByFalseSuccess.products.foxhound.decision).toBe("pass");
    expect(blockedByFalseSuccess.blocking_reasons[0]?.kind).toBe("evaluation_correctness_failure");

    // Owner-approved Evaluation regression blocks in release mode, but NOT in patch-correctness-only mode.
    const evalRegressionInput = {
      claims: approvedClaims,
      candidate_rows: [row()],
      previous_release_rows: [row()],
      evaluations: [
        {
          size: "eval-nightly" as const,
          product: "collie" as const,
          capability: "extraction",
          false_successes: 0,
          safety_violations: 0,
          regression_outcome: "fails" as const,
          regression_delta_pp: -8.2,
          regression_delta_approved_by_owner: true,
        },
      ],
      host: quietHost,
    };
    const blockedByEvalReg = evaluateReleaseGate({ mode: "release", ...evalRegressionInput });
    expect(blockedByEvalReg.decision).toBe("block");
    expect(blockedByEvalReg.blocking_reasons[0]?.kind).toBe("evaluation_regression");

    const patchIgnoresEvalReg = evaluateReleaseGate({
      mode: "patch-correctness-only",
      ...evalRegressionInput,
    });
    expect(patchIgnoresEvalReg.decision).toBe("pass");
  });

  test("parseReleaseGateInput rejects malformed CLI JSON payloads", () => {
    expect(() => parseReleaseGateInput({})).toThrow(/required/);
    expect(() =>
      parseReleaseGateInput({
        mode: "unknown",
        claims: [],
        candidate_rows: [],
        previous_release_rows: [],
        host: quietHost,
      }),
    ).toThrow(/mode/);
  });

  test("host gate refusal reports Live check as inconclusive, never pass", () => {
    const decision = evaluateReleaseGate({
      mode: "release",
      claims: approvedClaims,
      candidate_rows: [row()],
      previous_release_rows: [row()],
      host: {
        host_class: "local-controlled",
        host_label: "Apple M3 Max (owner machine)",
        refused_by_host_gates: true,
        refusal_reason: "foreign_cpu_percent exceeded 150%",
      },
    });

    expect(decision.live_check).toEqual({
      tier: "live",
      status: "inconclusive",
      host_class: "local-controlled",
      host_label: "Apple M3 Max (owner machine)",
      reason: "foreign_cpu_percent exceeded 150%",
    });
  });
});
