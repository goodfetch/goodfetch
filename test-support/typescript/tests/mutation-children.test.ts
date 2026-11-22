// StrykerJS runs reach the Node programs a test starts (ADR 0072, #3199):
// each runs the active mutant, and what it covers while StrykerJS learns
// which tests cover each mutant counts for the test that started it.
import { describe, expect } from "vitest";
import { ProcessRecord, test } from "@goodfetch/test-support";
import { reachMutationChild, type StrykerNamespace } from "../src/mutation-children.ts";

// The instrumented code's coverage step, as StrykerJS's instrumenter writes
// it at the head of each mutated file: it counts each mutant id it is given,
// for the running test when one is.
const COVERS = `
const ns = globalThis.__stryker__ || (globalThis.__stryker__ = {});
const cov = ns.mutantCoverage || (ns.mutantCoverage = { static: {}, perTest: {} });
function cover(...ids) {
  let c = cov.static;
  if (ns.currentTestId) {
    c = cov.perTest[ns.currentTestId] = cov.perTest[ns.currentTestId] || {};
  }
  for (const id of ids) c[id] = (c[id] || 0) + 1;
}
cover("7", "9");
cover("7");
`;

async function run(namespace: StrykerNamespace): Promise<void> {
  const processes = new ProcessRecord();
  const child = reachMutationChild(process.env, namespace);
  const started = processes.spawn(process.execPath, ["-e", COVERS], { env: child.env });
  const exit = await started.exit;
  child.ended();
  await processes.close();
  expect(exit).toMatchObject({ code: 0 });
}

describe("reachMutationChild", () => {
  test("counts what a program covers for the test that started it", async () => {
    const namespace: StrykerNamespace = { currentTestId: "a.test.ts#starts a program" };

    await run(namespace);

    expect(namespace.mutantCoverage).toEqual({
      static: {},
      perTest: { "a.test.ts#starts a program": { "7": 1, "9": 1 } },
    });
  });

  test("counts what a program started outside a test covers for every test", async () => {
    const namespace: StrykerNamespace = {
      mutantCoverage: { static: { "3": 2 }, perTest: {} },
    };

    await run(namespace);

    expect(namespace.mutantCoverage).toEqual({
      static: { "3": 2, "7": 1, "9": 1 },
      perTest: {},
    });
  });

  test("starts a program with the active mutant", () => {
    const child = reachMutationChild({ PATH: "/bin" }, { activeMutant: "12" });

    expect(child.env).toEqual({ PATH: "/bin", __STRYKER_ACTIVE_MUTANT__: "12" });
  });

  test("leaves a program outside a StrykerJS run as it is", () => {
    const env = { PATH: "/bin" };

    expect(reachMutationChild(env, undefined).env).toBe(env);
  });
});
