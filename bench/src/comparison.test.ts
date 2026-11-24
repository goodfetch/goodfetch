import { expect, test } from "vitest";
import { countStatuses } from "./cell.ts";
import { evaluateDecision, type ComparedCell, type ComparedRun } from "./comparison.ts";
import type { AttemptStatus } from "./errors.ts";
import type { Decision } from "./registry.ts";

const decision: Decision = {
  id: "D-driver",
  title: "Driver",
  question: "Which driver?",
  status: "open",
  declared_at: "2026-11-01T00:00:00.000Z",
  options: [
    { id: "node", description: "Node", adapter: "node" },
    { id: "rust", description: "Rust", adapter: "rust" },
    { id: "go", description: "Go", adapter: "go" },
  ],
  baseline: "node",
  primary_metric: "task_p50_ms_verified",
  secondary_metrics: [],
  contract_gates: ["oracle_exact", "false_success_count"],
  delta: 0.1,
  rule: { kind: "superiority", alpha: 0.05, power: 0.8, noisy_blocks: "exclude" },
  host_class: { decision: "local-controlled", publication: "reference" },
  evidence: ["r1"],
  depends_on: [],
  notes: "",
};

/** Each adapter's p50 on each task in each block: the baseline at 100 ms, `rust` at about half, `go` at about 70%. */
const speeds: Readonly<Record<string, number>> = { node: 100, rust: 50, go: 70 };

function cell(
  adapter: string,
  task: string,
  block: number,
  statuses: readonly AttemptStatus[] = ["verified", "verified", "verified"],
): ComparedCell {
  const speed = speeds[adapter] ?? 100;
  const jitter = 1 + ((block % 3) - 1) * 0.01;
  return {
    adapter,
    task,
    block,
    gate: "passed",
    counts: countStatuses(statuses.map((status) => ({ status }))),
    metrics: { task_p50_ms_verified: { value: speed * jitter } },
  };
}

function run(options: {
  readonly adapters?: readonly string[];
  readonly tasks?: readonly string[];
  readonly noisy?: readonly number[];
  readonly status?: (adapter: string, task: string) => readonly AttemptStatus[];
}): ComparedRun {
  const { adapters = ["node", "rust", "go"], tasks = ["catalog/1"], noisy = [] } = options;
  const blocks = Array.from({ length: 8 }, (_, block) => ({ block, noisy: noisy.includes(block) }));
  return {
    run_id: "r1",
    host: { class_granted: "local-controlled" },
    blocks,
    cells: blocks.flatMap(({ block }) =>
      adapters.flatMap((adapter) =>
        tasks.map((task) => cell(adapter, task, block, options.status?.(adapter, task))),
      ),
    ),
  };
}

test("pairs each option with the baseline by block and task, and judges it on its own gates", () => {
  const verdict = evaluateDecision(decision, [run({})]);

  expect(verdict.comparisons.map((one) => [one.option, one.outcome, one.n])).toEqual([
    ["rust", "better", 8],
    ["go", "better", 8],
  ]);
  expect(verdict.comparisons[0]?.estimate?.ratio).toBeCloseTo(0.5, 2);
  expect(verdict.comparisons[0]?.gates).toEqual([
    { id: "oracle_exact", ok: true },
    { id: "false_success_count", ok: true },
  ]);
  expect(verdict.comparisons[0]?.host).toEqual({
    required: "local-controlled",
    observed: "local-controlled",
    eligible: true,
  });
  expect(verdict.direction).toBe("lower");
});

test("a false success blocks the option whatever its speed, and only that option", () => {
  const verdict = evaluateDecision(decision, [
    run({
      status: (adapter) =>
        adapter === "rust" ? ["verified", "false_success", "verified"] : ["verified"],
    }),
  ]);

  expect(verdict.comparisons.map((one) => [one.option, one.outcome])).toEqual([
    ["rust", "blocked-by-contract"],
    ["go", "better"],
  ]);
  expect(verdict.comparisons[0]?.gates).toEqual([
    { id: "oracle_exact", ok: false },
    { id: "false_success_count", ok: false },
  ]);
});

test("an option absent from the runs fails its gates even when the baseline passes them", () => {
  const verdict = evaluateDecision(decision, [run({ adapters: ["node", "go"] })]);

  expect(verdict.comparisons[0]).toMatchObject({
    option: "rust",
    outcome: "blocked-by-contract",
    n: 0,
  });
});

test("leaves noisy blocks out of the estimate, or makes the verdict inconclusive, as the rule says", () => {
  const excluded = evaluateDecision(decision, [run({ noisy: [0, 3] })]);
  const flagged = evaluateDecision(
    { ...decision, rule: { ...decision.rule, noisy_blocks: "inconclusive" } },
    [run({ noisy: [0, 3] })],
  );

  expect(excluded.comparisons[0]).toMatchObject({ outcome: "better", n: 6 });
  expect(flagged.comparisons[0]).toMatchObject({ outcome: "inconclusive", n: 8 });
});

test("gives each declared stratum its own comparison, so opposite effects are not pooled", () => {
  const verdict = evaluateDecision(
    {
      ...decision,
      strata: [
        { id: "form", tasks: ["form/1"] },
        { id: "catalog", tasks: ["catalog/1"] },
      ],
    },
    [run({ tasks: ["catalog/1", "catalog/2"] })],
  );

  expect(verdict.comparisons.map((one) => [one.option, one.stratum, one.n])).toEqual([
    ["rust", "form", 0],
    ["rust", "catalog", 8],
    ["go", "form", 0],
    ["go", "catalog", 8],
  ]);
});

test("reads only the runs the decision cites, and names those it lacks", () => {
  const verdict = evaluateDecision({ ...decision, evidence: ["r1", "r2"] }, [
    run({}),
    { ...run({}), run_id: "r3" },
  ]);

  expect(verdict.inputs).toEqual({ runs: ["r1"], missing: ["r2"] });
  expect(verdict.comparisons[0]?.runs).toEqual(["r1"]);
  expect(verdict.comparisons[0]?.n).toBe(8);
});

test("marks a verdict from a host below the decision's class as not eligible", () => {
  const verdict = evaluateDecision(decision, [
    { ...run({}), host: { class_granted: "local-dev" } },
  ]);

  expect(verdict.comparisons[0]?.host).toEqual({
    required: "local-controlled",
    observed: "local-dev",
    eligible: false,
  });
});
