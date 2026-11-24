import { expect, test } from "vitest";
import {
  buildLedger,
  parseClassification,
  type Classification,
  type ClassificationEntry,
  type LedgerRun,
} from "./ledger.ts";

const run = (runId: string, overrides: Partial<LedgerRun> = {}): LedgerRun => ({
  run_id: runId,
  started_at: "2026-11-02T00:00:00.000Z",
  status: "complete",
  campaign: { id: "stubs" },
  host: { class_granted: "local-dev" },
  declared: {
    decisions: [{ id: "D-core-language", declared_at: "2026-11-01T00:00:00.000Z" }],
    claims: [],
  },
  ...overrides,
});
const entry = (overrides: Partial<ClassificationEntry> = {}): ClassificationEntry => ({
  role: "selected",
  reason: "cited result",
  superseded_by: null,
  decisions: ["D-core-language"],
  validity: [{ scope: "all", class: "B", note: "shared host" }],
  noisy_host: false,
  rerun: { required: false, priority: null, host_class: null },
  ...overrides,
});
const classification = (runs: Record<string, ClassificationEntry>): Classification => ({
  schema_version: 1,
  runs,
});
const decisions = ["D-core-language", "D-storage"];

test("every classified run must exist, and an unclassified run enters as unreviewed with its declared decisions", () => {
  const { errors, ledger } = buildLedger({
    runs: [run("a"), run("b")],
    classification: classification({ a: entry(), ghost: entry() }),
    citations: { a: ["reports/x.md"] },
    decisions,
  });

  expect(errors).toEqual(["ghost: classified but not a result run"]);
  expect(ledger.entries.find((one) => one.run_id === "b")).toMatchObject({
    role: "unreviewed",
    decisions: ["D-core-language"],
  });
});

test("a selected run must be cited, and a superseded run must point to an existing replacement", () => {
  const { errors } = buildLedger({
    runs: [run("a"), run("b")],
    classification: classification({
      a: entry(),
      b: entry({ role: "superseded", superseded_by: "missing-run" }),
    }),
    citations: {},
    decisions,
  });

  expect(errors).toEqual([
    "a: role selected but not cited in any report",
    "b: superseded by missing-run, which is not a result run",
  ]);
});

test("classification overlays decisions and the noisy-host status onto the ledger", () => {
  const { errors, ledger } = buildLedger({
    runs: [run("a"), run("b")],
    classification: classification({
      a: entry({
        noisy_host: true,
        rerun: { required: true, priority: "P0", host_class: "reference" },
      }),
      b: entry({ role: "diagnostic", reason: "interrupted build", decisions: ["D-storage"] }),
    }),
    citations: { a: ["reports/x.md", "reports/x.md"] },
    decisions,
  });

  expect(errors).toEqual([]);
  expect(ledger.entries.map((one) => [one.run_id, one.status, one.decisions])).toEqual([
    ["a", "noisy_host", ["D-core-language"]],
    ["b", "complete", ["D-storage"]],
  ]);
  expect(ledger.entries.find((one) => one.run_id === "a")).toMatchObject({
    status: "noisy_host",
    cited_in: ["reports/x.md"],
    rerun: { priority: "P0" },
  });
  expect(ledger.classification_sha256).toMatch(/^[0-9a-f]{64}$/);
});

test("a selected run needs a validity class, and decision IDs must exist in the registry", () => {
  const { errors } = buildLedger({
    runs: [run("a")],
    classification: classification({ a: entry({ validity: [], decisions: ["D-nope"] }) }),
    citations: { a: ["r.md"] },
    decisions,
  });

  expect(errors).toEqual(["a: unknown decision D-nope", "a: selected without a validity class"]);
});

test("reads a classification against its schema", () => {
  const valid = classification({ a: entry() });

  expect(parseClassification(valid, "classification.json")).toEqual(valid);
  expect(() =>
    parseClassification(
      { schema_version: 1, runs: { a: { ...entry(), role: "chosen" } } },
      "classification.json",
    ),
  ).toThrow(/classification\.json: .*role/);
});
