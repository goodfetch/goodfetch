// Release-gate decision (ADRs 0025, 0030, 0032, 0064, 0065, 0076, 0094):
// compares a candidate's benchmark and Evaluation results against the previous
// release per Product (`collie` and `foxhound`), and outputs `pass` or `block`
// with reasons, plus a Live check record and the list of rows where a Product
// is behind a competitor for owner acceptance before promotion (#2252 Q4).
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { BenchError } from "./errors.ts";
import type { EvaluationSize, Product } from "./evaluations.ts";
import { hostClasses, type HostClass } from "./host.ts";
import { metricCatalog, type MetricId } from "./metrics.ts";
import { parserOf } from "./schema.ts";
import type { RegressionOutcome } from "./statistics.ts";

/** Benchmark families defined in ADR 0025. */
export const BENCHMARK_FAMILIES = [
  "http-crawl",
  "rendered-crawl",
  "extraction",
  "scripted-actions",
  "remote-browser",
  "ai",
  "repeated-runs-drift",
  "incident-recovery",
  "startup-binding",
  "run-state-and-admission",
  "control-cost",
  "access",
] as const;

export type BenchmarkFamily = (typeof BENCHMARK_FAMILIES)[number];

/** Returns the smallest Product that carries `family` (ADR 0094). */
export function smallestProductForFamily(family: BenchmarkFamily): Product {
  switch (family) {
    case "http-crawl":
    case "rendered-crawl":
    case "incident-recovery":
    case "run-state-and-admission":
      return "foxhound";
    case "extraction":
    case "scripted-actions":
    case "remote-browser":
    case "ai":
    case "repeated-runs-drift":
    case "startup-binding":
    case "control-cost":
    case "access":
      return "collie";
    default: {
      const exhaustive: never = family;
      return exhaustive;
    }
  }
}

/** One family's declared release-gate claim (ADR 0025). */
export interface ReleaseGateClaim {
  readonly id: string;
  readonly family: BenchmarkFamily;
  readonly product: Product;
  readonly primary_metric: MetricId;
  /** Minimum effect Δ (positive ratio), or `null` while awaiting reference host spread (ADR 0076). */
  readonly delta: number | null;
  readonly delta_approved_by_owner: boolean;
  readonly declared_at: string;
  readonly notes: string;
}

/** File format for `bench/release-gate/claims.json`. */
export interface ReleaseGateClaimsFile {
  readonly schema_version: 1;
  readonly claims: readonly ReleaseGateClaim[];
}

const instant = {
  type: "string",
  pattern: "^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}(\\.\\d+)?Z$",
};

const releaseGateClaimsParser = parserOf<ReleaseGateClaimsFile>({
  type: "object",
  additionalProperties: false,
  required: ["schema_version", "claims"],
  properties: {
    schema_version: { const: 1 },
    claims: {
      type: "array",
      items: {
        type: "object",
        additionalProperties: false,
        required: [
          "id",
          "family",
          "product",
          "primary_metric",
          "delta",
          "delta_approved_by_owner",
          "declared_at",
          "notes",
        ],
        properties: {
          id: { type: "string", pattern: "^RG-[a-z0-9-]+$" },
          family: { enum: [...BENCHMARK_FAMILIES] },
          product: { enum: ["collie", "foxhound"] },
          primary_metric: { enum: Object.keys(metricCatalog) },
          delta: { anyOf: [{ type: "number", exclusiveMinimum: 0 }, { type: "null" }] },
          delta_approved_by_owner: { type: "boolean" },
          declared_at: instant,
          notes: { type: "string" },
        },
      },
    },
  },
});

/** Validates a release-gate claims document at the boundary. */
export function parseReleaseGateClaims(
  raw: unknown,
  label = "release-gate claims",
): ReleaseGateClaimsFile {
  return releaseGateClaimsParser.parse(raw, label);
}

/** Loads the declared release-gate claims from `filePath` (defaults to `bench/release-gate/claims.json`). */
export function loadReleaseGateClaims(filePath?: string): ReleaseGateClaimsFile {
  const target =
    filePath === undefined
      ? fileURLToPath(new URL("../release-gate/claims.json", import.meta.url))
      : filePath;
  const raw: unknown = JSON.parse(readFileSync(target, "utf8"));
  return parseReleaseGateClaims(raw, target);
}

/** One family's benchmark result for either a Product or a competitor. */
export interface FamilyBenchmarkRow {
  readonly family: BenchmarkFamily;
  /** Product when this is a Goodfetch row, or `null` for a competitor row. */
  readonly product: Product | null;
  readonly adapter: string;
  readonly is_competitor: boolean;
  readonly gate_passed: boolean;
  readonly verified: number;
  readonly false_success: number;
  readonly failed: number;
  readonly incomplete: number;
  readonly timeout: number;
  readonly crashed: number;
  readonly primary_metric_value: number | null;
}

/** Declared explanation for a primary-metric regression, carried into release notes (ADR 0025). */
export interface DeclaredRegressionExplanation {
  readonly product: Product;
  readonly family: BenchmarkFamily;
  readonly metric: MetricId;
  readonly explanation: string;
}

/** One Evaluation result observed since the previous release (ADR 0065). */
export interface EvaluationGateInput {
  readonly size: EvaluationSize;
  readonly product: Product;
  readonly capability: string;
  readonly false_successes: number;
  readonly safety_violations: number;
  readonly regression_outcome: RegressionOutcome;
  readonly regression_delta_pp: number | null;
  readonly regression_delta_approved_by_owner: boolean;
}

/** Row where a Product is behind a competitor, surfaced for owner acceptance (#2252 Q4). */
export interface BehindCompetitorInput {
  readonly product: Product;
  readonly family: BenchmarkFamily;
  readonly competitor: string;
  readonly metric: MetricId;
  /** Relative or absolute gap description (e.g. "-12.4%"). */
  readonly gap: string;
  readonly measured_cause: string;
  readonly next_cut: string;
}

/** Host gate status of the benchmark run feeding the Live check (ADR 0064). */
export interface HostRunStatus {
  readonly host_class: HostClass;
  readonly host_label: string;
  readonly refused_by_host_gates: boolean;
  readonly refusal_reason?: string | undefined;
}

/** Release-gate mode: full minor/major gate or patch correctness-only mode (ADRs 0030, 0032). */
export type ReleaseGateMode = "release" | "patch-correctness-only";

/** Input to `evaluateReleaseGate`. */
export interface ReleaseGateInput {
  readonly mode: ReleaseGateMode;
  readonly claims: readonly ReleaseGateClaim[];
  readonly candidate_rows: readonly FamilyBenchmarkRow[];
  readonly previous_release_rows: readonly FamilyBenchmarkRow[];
  readonly explanations?: readonly DeclaredRegressionExplanation[] | undefined;
  readonly evaluations?: readonly EvaluationGateInput[] | undefined;
  /** Optional owner-declared cause/next-cut annotations for behind-competitor rows computed from `candidate_rows`. */
  readonly behind_competitors?: readonly BehindCompetitorInput[] | undefined;
  readonly host: HostRunStatus;
}

const familyBenchmarkRowSchema = {
  type: "object",
  additionalProperties: false,
  required: [
    "family",
    "product",
    "adapter",
    "is_competitor",
    "gate_passed",
    "verified",
    "false_success",
    "failed",
    "incomplete",
    "timeout",
    "crashed",
    "primary_metric_value",
  ],
  properties: {
    family: { enum: [...BENCHMARK_FAMILIES] },
    product: { anyOf: [{ enum: ["collie", "foxhound"] }, { type: "null" }] },
    adapter: { type: "string", minLength: 1 },
    is_competitor: { type: "boolean" },
    gate_passed: { type: "boolean" },
    verified: { type: "integer", minimum: 0 },
    false_success: { type: "integer", minimum: 0 },
    failed: { type: "integer", minimum: 0 },
    incomplete: { type: "integer", minimum: 0 },
    timeout: { type: "integer", minimum: 0 },
    crashed: { type: "integer", minimum: 0 },
    primary_metric_value: { anyOf: [{ type: "number" }, { type: "null" }] },
  },
};

const releaseGateClaimSchema = {
  type: "object",
  additionalProperties: false,
  required: [
    "id",
    "family",
    "product",
    "primary_metric",
    "delta",
    "delta_approved_by_owner",
    "declared_at",
    "notes",
  ],
  properties: {
    id: { type: "string", pattern: "^RG-[a-z0-9-]+$" },
    family: { enum: [...BENCHMARK_FAMILIES] },
    product: { enum: ["collie", "foxhound"] },
    primary_metric: { enum: Object.keys(metricCatalog) },
    delta: { anyOf: [{ type: "number", exclusiveMinimum: 0 }, { type: "null" }] },
    delta_approved_by_owner: { type: "boolean" },
    declared_at: instant,
    notes: { type: "string" },
  },
};

const releaseGateInputParser = parserOf<ReleaseGateInput>({
  type: "object",
  additionalProperties: false,
  required: ["mode", "claims", "candidate_rows", "previous_release_rows", "host"],
  properties: {
    mode: { enum: ["release", "patch-correctness-only"] },
    claims: { type: "array", items: releaseGateClaimSchema },
    candidate_rows: { type: "array", items: familyBenchmarkRowSchema },
    previous_release_rows: { type: "array", items: familyBenchmarkRowSchema },
    explanations: {
      type: "array",
      items: {
        type: "object",
        additionalProperties: false,
        required: ["product", "family", "metric", "explanation"],
        properties: {
          product: { enum: ["collie", "foxhound"] },
          family: { enum: [...BENCHMARK_FAMILIES] },
          metric: { enum: Object.keys(metricCatalog) },
          explanation: { type: "string", minLength: 1 },
        },
      },
    },
    evaluations: {
      type: "array",
      items: {
        type: "object",
        additionalProperties: false,
        required: [
          "size",
          "product",
          "capability",
          "false_successes",
          "safety_violations",
          "regression_outcome",
          "regression_delta_pp",
          "regression_delta_approved_by_owner",
        ],
        properties: {
          size: { enum: ["eval-pr", "eval-nightly", "eval-weekly"] },
          product: { enum: ["collie", "foxhound"] },
          capability: { type: "string", minLength: 1 },
          false_successes: { type: "integer", minimum: 0 },
          safety_violations: { type: "integer", minimum: 0 },
          regression_outcome: { enum: ["passes", "fails", "inconclusive"] },
          regression_delta_pp: { anyOf: [{ type: "number" }, { type: "null" }] },
          regression_delta_approved_by_owner: { type: "boolean" },
        },
      },
    },
    behind_competitors: {
      type: "array",
      items: {
        type: "object",
        additionalProperties: false,
        required: [
          "product",
          "family",
          "competitor",
          "metric",
          "gap",
          "measured_cause",
          "next_cut",
        ],
        properties: {
          product: { enum: ["collie", "foxhound"] },
          family: { enum: [...BENCHMARK_FAMILIES] },
          competitor: { type: "string", minLength: 1 },
          metric: { enum: Object.keys(metricCatalog) },
          gap: { type: "string", minLength: 1 },
          measured_cause: { type: "string", minLength: 1 },
          next_cut: { type: "string", minLength: 1 },
        },
      },
    },
    host: {
      type: "object",
      additionalProperties: false,
      required: ["host_class", "host_label", "refused_by_host_gates"],
      properties: {
        host_class: { enum: [...hostClasses] },
        host_label: { type: "string", minLength: 1 },
        refused_by_host_gates: { type: "boolean" },
        refusal_reason: { type: "string", minLength: 1 },
      },
    },
  },
});

/** Validates untrusted `bench release-gate` CLI input at the boundary. */
export function parseReleaseGateInput(
  raw: unknown,
  label = "release-gate input",
): ReleaseGateInput {
  return releaseGateInputParser.parse(raw, label);
}

/** One blocking or reported reason from the release gate. */
export interface GateReason {
  readonly product: Product;
  readonly kind:
    | "correctness_failure"
    | "primary_metric_regression"
    | "evaluation_correctness_failure"
    | "evaluation_regression"
    | "patch_product_mismatch";
  readonly family_or_capability: string;
  readonly message: string;
}

/** Per-Product release-gate decision. */
export interface ProductReleaseDecision {
  readonly product: Product;
  readonly decision: "pass" | "block";
  readonly blocking_reasons: readonly GateReason[];
  readonly reported_changes: readonly string[];
  readonly release_notes_explanations: readonly DeclaredRegressionExplanation[];
}

/** Live check result written to the results store's `live` tier (ADR 0064). */
export interface LiveCheckReport {
  readonly tier: "live";
  readonly status: "pass" | "fail" | "inconclusive";
  readonly host_class: HostClass;
  readonly host_label: string;
  readonly reason: string;
}

/** Complete release-gate decision across both Products (ADR 0094). */
export interface ReleaseGateDecision {
  readonly mode: ReleaseGateMode;
  /** `pass` only when BOTH Products pass; a block in either blocks the release (ADR 0094). */
  readonly decision: "pass" | "block";
  readonly products: Readonly<Record<Product, ProductReleaseDecision>>;
  readonly blocking_reasons: readonly GateReason[];
  readonly release_notes_explanations: readonly DeclaredRegressionExplanation[];
  /** Rows where a Product is behind a competitor for owner acceptance; never blocks (#2252 Q4). */
  readonly behind_competitors: readonly BehindCompetitorInput[];
  readonly live_check: LiveCheckReport;
}

function hasCorrectnessFailure(row: FamilyBenchmarkRow): boolean {
  return (
    !row.gate_passed ||
    row.false_success > 0 ||
    row.failed > 0 ||
    row.incomplete > 0 ||
    row.timeout > 0 ||
    row.crashed > 0
  );
}

function relativeWorsening(
  candidate: number,
  previous: number,
  direction: "higher" | "lower",
): number {
  if (previous === 0) {
    return direction === "lower" ? (candidate > 0 ? Number.POSITIVE_INFINITY : 0) : 0;
  }
  if (direction === "higher") {
    return (previous - candidate) / Math.abs(previous);
  }
  return (candidate - previous) / Math.abs(previous);
}

function formatGap(worsening: number, direction: "higher" | "lower"): string {
  const pct = (worsening * 100).toFixed(1);
  return direction === "higher" ? `-${pct}%` : `+${pct}%`;
}

/** Validates non-empty trimmed metadata on any caller-supplied behind-competitor notes. */
function validateBehindCompetitorNote(note: BehindCompetitorInput): void {
  if (
    note.gap.trim().length === 0 ||
    note.measured_cause.trim().length === 0 ||
    note.next_cut.trim().length === 0
  ) {
    throw new BenchError(
      "invalid_document",
      `behind_competitors entry for ${note.product}/${note.family} vs ${note.competitor} requires non-empty gap, measured_cause and next_cut`,
    );
  }
}

/** Evaluates a release candidate against its previous release per Product. */
export function evaluateReleaseGate(input: ReleaseGateInput): ReleaseGateDecision {
  const claimByKey = new Map<string, ReleaseGateClaim>(
    input.claims.map((claim) => [`${claim.product}:${claim.family}`, claim]),
  );

  const suppliedBehind = new Map<string, BehindCompetitorInput>();
  for (const note of input.behind_competitors ?? []) {
    validateBehindCompetitorNote(note);
    suppliedBehind.set(`${note.product}:${note.family}:${note.competitor}:${note.metric}`, note);
  }

  const perProduct: Record<
    Product,
    {
      blocking: GateReason[];
      reported: string[];
      explanations: DeclaredRegressionExplanation[];
    }
  > = {
    collie: { blocking: [], reported: [], explanations: [] },
    foxhound: { blocking: [], reported: [], explanations: [] },
  };

  // 1. Evaluate candidate benchmark rows (competitor rows NEVER block).
  const competitorRowsByFamily = new Map<BenchmarkFamily, FamilyBenchmarkRow[]>();
  for (const row of input.candidate_rows) {
    if (row.is_competitor) {
      const list = competitorRowsByFamily.get(row.family) ?? [];
      list.push(row);
      competitorRowsByFamily.set(row.family, list);
    }
  }

  const computedBehind: BehindCompetitorInput[] = [];

  for (const row of input.candidate_rows) {
    if (row.is_competitor || row.product === null) {
      continue;
    }
    const product = row.product;
    const bucket = perProduct[product];

    // Patch correctness-only mode verifies each family runs on the smallest Product (ADR 0094).
    if (input.mode === "patch-correctness-only") {
      const smallest = smallestProductForFamily(row.family);
      if (product !== smallest) {
        bucket.blocking.push({
          product,
          kind: "patch_product_mismatch",
          family_or_capability: row.family,
          message: `patch correctness-only mode requires family ${row.family} on ${smallest}, got ${product}`,
        });
      }
    }

    // Correctness failure always blocks.
    if (hasCorrectnessFailure(row)) {
      bucket.blocking.push({
        product,
        kind: "correctness_failure",
        family_or_capability: row.family,
        message: `${product} failed correctness on ${row.family} (gate_passed=${row.gate_passed}, false_success=${row.false_success}, failed=${row.failed}, incomplete=${row.incomplete}, timeout=${row.timeout}, crashed=${row.crashed})`,
      });
    }

    if (input.mode === "patch-correctness-only") {
      continue;
    }

    // Primary-metric lookup is strictly per (product, family) — never cross-match another Product's claim.
    const claim = claimByKey.get(`${product}:${row.family}`);

    // Compute behind-competitor rows directly from `candidate_rows` (#2252 Q4); competitor rows never block.
    if (claim !== undefined && row.primary_metric_value !== null) {
      const { direction } = metricCatalog[claim.primary_metric];
      for (const comp of competitorRowsByFamily.get(row.family) ?? []) {
        if (!comp.gate_passed || comp.primary_metric_value === null) {
          continue;
        }
        const worsening = relativeWorsening(
          row.primary_metric_value,
          comp.primary_metric_value,
          direction,
        );
        if (worsening > 0 && Number.isFinite(worsening)) {
          const note = suppliedBehind.get(
            `${product}:${row.family}:${comp.adapter}:${claim.primary_metric}`,
          );
          computedBehind.push({
            product,
            family: row.family,
            competitor: comp.adapter,
            metric: claim.primary_metric,
            gap: formatGap(worsening, direction),
            measured_cause: note?.measured_cause ?? "unmeasured — cause required before promotion",
            next_cut: note?.next_cut ?? "undeclared — next cut required before promotion",
          });
        }
      }
    }

    // Primary-metric comparison against previous release.
    const previous = input.previous_release_rows.find(
      (prev) => !prev.is_competitor && prev.product === product && prev.family === row.family,
    );
    if (
      claim === undefined ||
      previous === undefined ||
      row.primary_metric_value === null ||
      previous.primary_metric_value === null
    ) {
      continue;
    }
    const { direction } = metricCatalog[claim.primary_metric];
    const worsening = relativeWorsening(
      row.primary_metric_value,
      previous.primary_metric_value,
      direction,
    );
    if (worsening <= 0) {
      continue;
    }

    // If Δ is not yet set/approved (ADR 0076), report the change without blocking.
    if (claim.delta === null || !claim.delta_approved_by_owner) {
      bucket.reported.push(
        `${product}/${row.family} (${claim.primary_metric}): worse by ${(worsening * 100).toFixed(1)}% (unapproved Δ; reported only per ADR 0076)`,
      );
      continue;
    }

    if (worsening > claim.delta) {
      const declared = (input.explanations ?? []).find(
        (exp) =>
          exp.product === product &&
          exp.family === row.family &&
          exp.metric === claim.primary_metric,
      );
      if (declared !== undefined) {
        bucket.explanations.push(declared);
      } else {
        bucket.blocking.push({
          product,
          kind: "primary_metric_regression",
          family_or_capability: row.family,
          message: `${product}/${row.family} ${claim.primary_metric} regressed by ${(worsening * 100).toFixed(1)}% > Δ ${(claim.delta * 100).toFixed(1)}% with no declared explanation`,
        });
      }
    }
  }

  // 2. Evaluate Evaluation results of any size since the previous release (ADR 0065).
  for (const evalItem of input.evaluations ?? []) {
    const bucket = perProduct[evalItem.product];
    if (evalItem.false_successes > 0 || evalItem.safety_violations > 0) {
      bucket.blocking.push({
        product: evalItem.product,
        kind: "evaluation_correctness_failure",
        family_or_capability: `${evalItem.size}:${evalItem.capability}`,
        message: `${evalItem.size} (${evalItem.product}/${evalItem.capability}) reported false_successes=${evalItem.false_successes}, safety_violations=${evalItem.safety_violations}`,
      });
    }
    // Patch correctness-only mode checks correctness/safety only, never metric regressions.
    if (input.mode === "patch-correctness-only") {
      continue;
    }
    if (evalItem.regression_outcome === "fails") {
      if (evalItem.regression_delta_approved_by_owner) {
        bucket.blocking.push({
          product: evalItem.product,
          kind: "evaluation_regression",
          family_or_capability: `${evalItem.size}:${evalItem.capability}`,
          message: `${evalItem.size} (${evalItem.product}/${evalItem.capability}) regressed beyond owner-approved Δ (${evalItem.regression_delta_pp ?? "n/a"} pp)`,
        });
      } else {
        bucket.reported.push(
          `${evalItem.size} (${evalItem.product}/${evalItem.capability}): regression reported (Δ not yet owner-approved, ADR 0065)`,
        );
      }
    }
  }

  const collieDecision: ProductReleaseDecision = {
    product: "collie",
    decision: perProduct.collie.blocking.length === 0 ? "pass" : "block",
    blocking_reasons: perProduct.collie.blocking,
    reported_changes: perProduct.collie.reported,
    release_notes_explanations: perProduct.collie.explanations,
  };
  const foxhoundDecision: ProductReleaseDecision = {
    product: "foxhound",
    decision: perProduct.foxhound.blocking.length === 0 ? "pass" : "block",
    blocking_reasons: perProduct.foxhound.blocking,
    reported_changes: perProduct.foxhound.reported,
    release_notes_explanations: perProduct.foxhound.explanations,
  };

  const allBlocking = [...collieDecision.blocking_reasons, ...foxhoundDecision.blocking_reasons];
  const allExplanations = [
    ...collieDecision.release_notes_explanations,
    ...foxhoundDecision.release_notes_explanations,
  ];
  const overallDecision: "pass" | "block" = allBlocking.length === 0 ? "pass" : "block";

  // 3. Live check status (ADR 0064): host gate refusal is ALWAYS `inconclusive`, never `pass`.
  let liveCheck: LiveCheckReport;
  if (input.host.refused_by_host_gates) {
    liveCheck = {
      tier: "live",
      status: "inconclusive",
      host_class: input.host.host_class,
      host_label: input.host.host_label,
      reason: input.host.refusal_reason ?? "run refused by host gates",
    };
  } else if (overallDecision === "block") {
    liveCheck = {
      tier: "live",
      status: "fail",
      host_class: input.host.host_class,
      host_label: input.host.host_label,
      reason: `${allBlocking.length} blocking release-gate finding(s)`,
    };
  } else {
    liveCheck = {
      tier: "live",
      status: "pass",
      host_class: input.host.host_class,
      host_label: input.host.host_label,
      reason: "all release-gate checks passed",
    };
  }

  return {
    mode: input.mode,
    decision: overallDecision,
    products: {
      collie: collieDecision,
      foxhound: foxhoundDecision,
    },
    blocking_reasons: allBlocking,
    release_notes_explanations: allExplanations,
    behind_competitors: computedBehind,
    live_check: liveCheck,
  };
}
