// Seam 2 (spec #152, issue 174): a campaign that declares a decision and its
// claim before measuring, run by `bench run` against the commit's Proving
// ground with two stubs that differ only by their answer delay. The registry
// is checked before anything is measured, and the run record keeps what it
// declared, so a claim declared after the run is refused.
import { readdir } from "node:fs/promises";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { bench, written } from "./bench.ts";

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

  test("records the decisions and claims a run declared, as dated when it started", async ({
    provingGround,
  }) => {
    const ran = await bench("decisions.json", provingGround.address);
    expect(ran.stderr).toBe("");
    const { record } = await written(ran.out);

    expect(record.declared).toEqual({
      decisions: [{ id: "D-stub-speed", declared_at: declaredAt }],
      claims: [{ id: "C-stub-fast-lower", declared_at: declaredAt }],
    });
    expect(Date.parse(record.started_at)).toBeGreaterThan(Date.parse(declaredAt));
  });
});
