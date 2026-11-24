import { mkdir, mkdtemp, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { expect, test } from "vitest";
import {
  checkRegistry,
  loadRegistry,
  type Claim,
  type Decision,
  type DeclaredRun,
  type Registry,
} from "./registry.ts";

const decision = (overrides: Partial<Decision> = {}): Decision => ({
  id: "D-example",
  title: "Example",
  question: "Which option?",
  status: "open",
  declared_at: "2026-11-01T00:00:00.000Z",
  options: [
    { id: "a", description: "A", adapter: "a" },
    { id: "b", description: "B", adapter: "b" },
  ],
  baseline: "a",
  primary_metric: "cpu_ms_per_verified_task",
  secondary_metrics: [],
  contract_gates: ["oracle_exact"],
  delta: 0.1,
  rule: { kind: "superiority", alpha: 0.05, power: 0.8, noisy_blocks: "exclude" },
  host_class: { decision: "local-controlled", publication: "reference" },
  evidence: [],
  depends_on: [],
  notes: "",
  ...overrides,
});
const claim = (overrides: Partial<Claim> = {}): Claim => ({
  id: "C-example",
  statement: "Option b needs less CPU per verified task than a",
  decision: "D-example",
  metric: "cpu_ms_per_verified_task",
  comparator: { option: "b", against: ["a"] },
  direction: "lower",
  delta: 0.1,
  publication_host_class: "reference",
  status: "candidate",
  declared_at: "2026-11-01T00:00:00.000Z",
  evidence: [],
  notes: "",
  ...overrides,
});
const registry = (decisions: Decision[], claims: Claim[] = []): Registry => ({ decisions, claims });
const run = (overrides: Partial<DeclaredRun> = {}): DeclaredRun => ({
  run_id: "r1",
  started_at: "2026-11-02T00:00:00.000Z",
  declared: {
    decisions: [{ id: "D-example", declared_at: "2026-11-01T00:00:00.000Z" }],
    claims: [{ id: "C-example", declared_at: "2026-11-01T00:00:00.000Z" }],
  },
  ...overrides,
});

test("accepts a decision and its claim that name known metrics, gates and options", () => {
  expect(checkRegistry(registry([decision()], [claim()]))).toEqual([]);
});

test("a decision must name a baseline among its options and metrics and gates of the catalogue", () => {
  const errors = checkRegistry(
    registry([
      decision({ id: "D-bad-baseline", baseline: "z" }),
      decision({ id: "D-bad-option", options: [{ id: "a", description: "A", adapter: "a" }] }),
      decision({ id: "D-dupe" }),
      decision({ id: "D-dupe", depends_on: ["D-missing"] }),
    ]),
  );

  expect(errors).toContain("D-bad-baseline: baseline z is not one of its options");
  expect(errors).toContain("D-bad-option: a decision compares at least two options");
  expect(errors).toContain("D-dupe: duplicate decision id");
  expect(errors).toContain("D-dupe: depends on unknown decision D-missing");
});

test("editorial notes cannot carry measured figures, which must come from verdicts", () => {
  const errors = checkRegistry(
    registry(
      [decision({ notes: "Rust was 49 % faster at 197 tasks/s" })],
      [claim({ statement: "b is 3x faster" })],
    ),
  );

  expect(errors).toContain(
    "D-example: notes contain a measured figure (49 %); figures must come from verdicts",
  );
  expect(errors).toContain(
    "C-example: statement or notes contain a measured figure (3x); figures must come from verdicts",
  );
});

test("a claim must resolve its decision, its options and a metric the decision measures", () => {
  const errors = checkRegistry(
    registry(
      [decision()],
      [
        claim({ id: "C-orphan", decision: "D-nowhere" }),
        claim({ id: "C-option", comparator: { option: "c", against: ["a"] } }),
        claim({ id: "C-metric", metric: "rss_peak_bytes" }),
        claim({ id: "C-delta", delta: 0.2 }),
      ],
    ),
  );

  expect(errors).toEqual([
    "C-orphan: refers to unknown decision D-nowhere",
    "C-option: option c is not an option of D-example",
    "C-metric: metric rss_peak_bytes is not measured by D-example",
    "C-delta: its Δ 0.2 is not D-example's 0.1 on the decision's primary metric",
  ]);
});

test("refuses a claim or decision declared after now, before anything is measured", () => {
  const errors = checkRegistry(
    registry([decision()], [claim({ declared_at: "2026-12-01T00:00:00.000Z" })]),
    { now: new Date("2026-11-15T00:00:00.000Z") },
  );

  expect(errors).toEqual(["C-example: declared at 2026-12-01T00:00:00.000Z, after now"]);
});

test("refuses a claim declared after its run, or never declared by it", () => {
  const late = claim({ declared_at: "2026-11-03T00:00:00.000Z", evidence: ["r1"] });
  const unknown = claim({ id: "C-unknown", evidence: ["r9"] });
  const errors = checkRegistry(registry([decision({ evidence: ["r2"] })], [late, unknown]), {
    runs: [run(), run({ run_id: "r2", declared: { decisions: [], claims: [] } })],
  });

  expect(errors).toEqual([
    "D-example: run r2 did not declare it before measuring",
    "C-example: declared at 2026-11-03T00:00:00.000Z, after its run r1 started at 2026-11-02T00:00:00.000Z",
    "C-example: declared at 2026-11-03T00:00:00.000Z, but run r1 recorded 2026-11-01T00:00:00.000Z",
    "C-unknown: evidence run r9 is not a known run",
  ]);
});

test("loads decisions and claims from their folders, each checked against its schema", async () => {
  const root = await mkdtemp(join(tmpdir(), "goodfetch-registry-"));
  await mkdir(join(root, "decisions"));
  await writeFile(join(root, "decisions", "D-example.json"), JSON.stringify(decision()));

  expect(await loadRegistry(root)).toEqual({ decisions: [decision()], claims: [] });

  await mkdir(join(root, "claims"));
  await writeFile(join(root, "claims", "C-example.json"), JSON.stringify({ id: "C-example" }));
  await expect(loadRegistry(root)).rejects.toThrow(/C-example\.json: .*statement/);
});
