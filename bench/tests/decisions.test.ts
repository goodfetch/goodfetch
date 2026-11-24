// Seam 2 (spec #152, issue 174): a campaign that declares a decision and its
// claim before measuring, run by `bench run` against the commit's Proving
// ground with two stubs that differ only by their answer delay. The registry
// is checked before anything is measured, and the run record keeps what it
// declared, so a claim declared after the run is refused. The decision's
// verdict follows mechanically from the record's paired blocks, and the
// statistics engine reads the verified rate, pass^k and the regression
// statistic from its attempts.
import { readdir } from "node:fs/promises";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { evaluateDecision } from "../src/comparison.ts";
import { checkRegistry, loadRegistry } from "../src/registry.ts";
import { attemptSummary, passK, regression, wilson } from "../src/statistics.ts";
import { bench, written } from "./bench.ts";

const registryFolder = new URL("campaigns/registry", import.meta.url).pathname;
const declaredAt = "2026-11-24T00:00:00.000Z";

describe("declared decisions", { tags: ["story:2-18"] }, () => {
  test.override("provingGroundSeed", 152);

  test("refuses a campaign declaring a decision the registry lacks before it measures or writes anything", async ({
    provingGround,
  }) => {
    const ran = await bench("undeclared.json", provingGround.address);

    expect({ code: ran.code, error: ran.stderr.split(":")[0] }).toEqual({
      code: 1,
      error: "error[registry_refused]",
    });
    expect(ran.stderr).toContain("the campaign declares unknown decision D-stub-undeclared");
    expect(await readdir(ran.out)).toEqual([]);
  });
});

describe("a measured decision", { tags: ["story:2-18", "story:2-19", "story:2-89"] }, () => {
  test.override("provingGroundSeed", 152);

  test("judges its declared claim mechanically from the run, and refuses the claim declared after it", async ({
    provingGround,
  }) => {
    const ran = await bench("decisions.json", provingGround.address);
    expect(ran.stderr).toBe("");
    const { record, attempts } = await written(ran.out);
    const registry = await loadRegistry(registryFolder);

    // Declared before measuring: the record keeps each declaration as it stood.
    expect(record.declared).toEqual({
      decisions: [{ id: "D-stub-speed", declared_at: declaredAt }],
      claims: [{ id: "C-stub-fast-lower", declared_at: declaredAt }],
    });
    const cited = {
      decisions: registry.decisions.map((one) => ({ ...one, evidence: [record.run_id] })),
      claims: registry.claims.map((one) => ({ ...one, evidence: [record.run_id] })),
    };
    expect(checkRegistry(cited, { runs: [record] })).toEqual([]);
    const late = new Date(Date.parse(record.started_at) + 1000).toISOString();
    expect(
      checkRegistry(
        { ...cited, claims: cited.claims.map((one) => ({ ...one, declared_at: late })) },
        { runs: [record] },
      ),
    ).toContain(
      `C-stub-fast-lower: declared at ${late}, after its run ${record.run_id} started at ${record.started_at}`,
    );

    // Five paired blocks of the one task, judged by the decision's fixed rule.
    const [decision] = cited.decisions;
    if (decision === undefined) {
      throw new Error("the fixture registry declares D-stub-speed");
    }
    const [primary] = evaluateDecision(decision, [record]).comparisons;
    expect({
      option: primary?.option,
      outcome: primary?.outcome,
      n: primary?.n,
      gates: primary?.gates.map((gate) => gate.ok),
      eligible: primary?.host.eligible,
    }).toEqual({ option: "fast", outcome: "better", n: 5, gates: [true, true], eligible: true });
    expect(primary?.estimate?.ratioCi[1]).toBeLessThan(1 / 2);

    // The statistics engine on the same attempts.
    const measured = (adapter: string) =>
      attempts.filter((one) => one.adapter === adapter && one.phase === "measure");
    const fast = attemptSummary(
      measured("fast").map((one) => ({ status: one.status, elapsedMs: one.timing.task_ms })),
    );
    const trials = (adapter: string) => ({
      trials: measured(adapter).length,
      verified: measured(adapter).filter((one) => one.status === "verified").length,
    });
    expect({ attempts: fast.attempts, rate: fast.verifiedRate }).toEqual({ attempts: 15, rate: 1 });
    expect(fast.zeroFailureUpper95).toBeCloseTo(1 - 0.05 ** (1 / 15), 12);
    // Wilson's lower bound at 15 of 15 is 15 / (15 + z²).
    expect(wilson(fast.counts.verified, fast.attempts).ci[0]).toBeCloseTo(0.796, 3);
    expect(passK([trials("fast")], 3)).toBe(1);
    expect(
      regression([{ task: "catalog/2", run: trials("fast"), baseline: trials("slow") }], {
        delta: 5,
        seed: 1,
      }),
    ).toMatchObject({ outcome: "insufficient-n", n: 1, estimatePp: 0 });
  });
});
