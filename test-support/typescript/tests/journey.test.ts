// The TypeScript Journey helper (ADR 0063; TESTING.md, Journeys): a run of a
// fixture Journey writes the result document `goodfetch-conformance read
// --journey` holds to that Journey's file. These tests start runs from
// fixture files, which carry IDs no catalog tag names, so they carry no tag
// and declare no Feature.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect } from "vitest";
import {
  startJourneyRun,
  test as base,
  toolPath,
  type JourneyRun,
  type ProcessRecord,
  type TestEnvDescription,
} from "@goodfetch/test-support";
import { catalogRecord } from "./catalog-product.ts";

/** The fixture Journey file `name`. */
function fixture(name: string): string {
  return fileURLToPath(new URL(`fixtures/journey-run/${name}`, import.meta.url));
}

/** `goodfetch-conformance read <document> --journey <file>`'s exit code: 0 when it accepts the document. */
async function read(
  processes: ProcessRecord,
  testEnv: TestEnvDescription,
  document: string,
  journey: string,
): Promise<number | null> {
  const reader = processes.spawn(
    toolPath(testEnv.tools, "conformance"),
    ["read", document, "--journey", fixture(journey)],
    { env: testEnv.env },
  );
  reader.child.stdin.end();
  return (await reader.exit).code;
}

/** The document a finished run wrote, read back from its file. */
function written(path: string): {
  steps: { text: string; outcome: string }[];
  checks: { text: string; task?: string; outcome: string }[];
  tasks: { id: string; answer?: { outcome: string } }[];
  proving_ground?: { seed: number };
  executable: { product: string };
} {
  return JSON.parse(readFileSync(path, "utf8"));
}

/** Each entry of a document's list as `<outcome> <text>`. */
function outcomes(entries: readonly { text: string; outcome: string }[]): string[] {
  return entries.map(({ text, outcome }) => `${outcome} ${text}`);
}

/** Asks the run's executable for its version, failing unless it answers. */
async function askVersion(run: JourneyRun): Promise<void> {
  const version = run.command(["--version"]);
  version.child.stdin.end();
  version.child.stdout.resume();
  version.child.stderr.resume();
  const exit = await version.exit;
  if (exit.code !== 0) {
    throw new Error(`--version exited with ${exit.code}`);
  }
}

interface JourneyRunFixtures {
  /** Starts a run of the fixture Journey `file` on `surface`; the test's end tears it down. */
  startRun: (
    file: string,
    surface: string,
    env?: Readonly<Record<string, string | undefined>>,
  ) => Promise<JourneyRun>;
}

const test = base.extend<JourneyRunFixtures>({
  startRun: async ({ processes, toolManifest, task }, use) => {
    const runs: JourneyRun[] = [];
    await use(async (file, surface, env) => {
      const run = await startJourneyRun({
        file: fixture(file),
        surface,
        env,
        processes,
        testName: task.fullName,
        toolManifest,
      });
      runs.push(run);
      return run;
    });
    for (const run of runs) {
      await run.close({ failed: task.result?.state === "fail" });
    }
  },
});

describe("startJourneyRun", () => {
  test("writes a proved document the reader accepts when every step passes and the oracle verifies", async ({
    startRun,
    processes,
  }) => {
    const run = await startRun("J-961.toml", "sdk-typescript");
    await run.step("Ask the executable for its version", () => askVersion(run));
    const record = await run.step("Read the product its task's start page shows", () =>
      catalogRecord(run.provingGround ?? { address: "" }, "catalog/2"),
    );
    await run.expectTask("The oracle verifies the product's Record", { records: [record ?? {}] });

    const finished = run.finish();

    expect(finished.outcome).toBe("proved");
    expect(finished.failures).toEqual([]);
    expect(() => {
      finished.assertProved();
    }).not.toThrow();
    expect(await read(processes, run.testEnv, finished.path, "J-961.toml")).toBe(0);
  });

  test("writes a document the reader refuses when a step is skipped, without running the step called early", async ({
    startRun,
    processes,
  }) => {
    const run = await startRun("J-960.toml", "sdk-typescript");
    let ran = false;
    await run.step("Ask the executable for its version", () => askVersion(run));
    await run.step("Ask the executable for its version once more", () => {
      ran = true;
    });

    const finished = run.finish();

    expect(ran).toBe(false);
    expect(outcomes(written(finished.path).steps)).toEqual([
      "passed Ask the executable for its version",
      "manual Read the version aloud",
      "failed Ask the executable for its version once more",
      "not_reached Ask the executable for its help",
    ]);
    expect(finished.failures).toEqual([
      "step `Ask the executable for its version once more` is called before step 3 `Ask the executable for its help`",
    ]);
    expect(await read(processes, run.testEnv, finished.path, "J-960.toml")).toBe(1);
  });

  test("writes a failed document the reader accepts when the oracle answers false_success", async ({
    startRun,
    processes,
  }) => {
    const run = await startRun("J-961.toml", "sdk-typescript");
    await run.step("Ask the executable for its version", () => askVersion(run));
    const record = await run.step("Read the product its task's start page shows", () =>
      catalogRecord(run.provingGround ?? { address: "" }, "catalog/2"),
    );
    const answer = await run.expectTask("The oracle verifies the product's Record", {
      records: [{ ...record, price: "0.01" }],
    });

    const finished = run.finish();

    expect(answer?.outcome).toBe("false_success");
    expect(finished.outcome).toBe("failed");
    expect(written(finished.path)).toMatchObject({
      checks: [{ task: "catalog/2", outcome: "failed" }],
      tasks: [{ id: "catalog/2", answer: { outcome: "false_success" } }],
    });
    expect(finished.failures).toEqual([
      "the oracle answers `false_success` for task `catalog/2` of check `The oracle verifies the product's Record`",
    ]);
    expect(() => {
      finished.assertProved();
    }).toThrow(
      expect.objectContaining({
        code: "variant_not_proved",
      }),
    );
    expect(await read(processes, run.testEnv, finished.path, "J-961.toml")).toBe(0);
  });

  test("starts no Proving ground for a Journey with no proving_ground table", async ({
    startRun,
  }) => {
    const run = await startRun("J-960.toml", "sdk-typescript");

    const finished = run.finish();

    expect(run.testEnv.proving_ground).toBeUndefined();
    expect(run.provingGround).toBeUndefined();
    expect(written(finished.path).proving_ground).toBeUndefined();
  });

  test("starts the Proving ground on the Journey's seed whatever GOODFETCH_TEST_SEED says", async ({
    startRun,
  }) => {
    const run = await startRun("J-961.toml", "sdk-typescript", {
      ...process.env,
      GOODFETCH_TEST_SEED: "4242",
    });

    const finished = run.finish();

    expect(run.testEnv.seed).toBe(4242);
    expect(run.provingGround?.seed).toBe(961);
    expect(written(finished.path).proving_ground?.seed).toBe(961);
  });

  test("records the steps after a failed step not reached and the manual step manual", async ({
    startRun,
    processes,
  }) => {
    const run = await startRun("J-960.toml", "sdk-typescript");
    await run.step("Ask the executable for its version", () => {
      throw new Error("no version");
    });
    let ran = false;
    await run.step("Ask the executable for its help", () => {
      ran = true;
    });
    await run.check("The version names the Product", () => {
      ran = true;
    });

    const finished = run.finish();

    expect(ran).toBe(false);
    const document = written(finished.path);
    expect(outcomes(document.steps)).toEqual([
      "failed Ask the executable for its version",
      "manual Read the version aloud",
      "not_reached Ask the executable for its help",
      "not_reached Ask the executable for its version once more",
    ]);
    expect(outcomes(document.checks)).toEqual(["not_reached The version names the Product"]);
    expect(finished.failures).toEqual([
      "step 1 `Ask the executable for its version` failed: no version",
    ]);
    expect(await read(processes, run.testEnv, finished.path, "J-960.toml")).toBe(0);
  });

  test("neither runs nor records a check of another variant's surface", async ({ startRun }) => {
    const run = await startRun("J-960.toml", "sdk-typescript");
    let ran = false;

    await run.check("The CLI prints the version on one line", () => {
      ran = true;
    });
    const finished = run.finish();

    expect(ran).toBe(false);
    expect(outcomes(written(finished.path).checks)).toEqual([
      "not_reached The version names the Product",
      "failed The CLI prints the version on one line",
    ]);
    expect(finished.failures).toEqual([
      "check `The CLI prints the version on one line` is not a check of J-960 that names no task",
    ]);
  });

  test("judges each check naming one task on its own submission", async ({
    startRun,
    processes,
  }) => {
    const run = await startRun("J-962.toml", "sdk-typescript");
    const record = await run.step("Read the product its task's start page shows, twice", () =>
      catalogRecord(run.provingGround ?? { address: "" }, "catalog/2"),
    );
    await run.expectTask("The oracle verifies the first read's Record", {
      records: [record ?? {}],
    });
    await run.expectTask("The oracle verifies the second read's Record", {
      records: [{ ...record, price: "0.01" }],
    });

    const finished = run.finish();

    expect(written(finished.path).tasks).toMatchObject([
      { id: "catalog/2", answer: { outcome: "verified" } },
      { id: "catalog/2", answer: { outcome: "false_success" } },
    ]);
    expect(await read(processes, run.testEnv, finished.path, "J-962.toml")).toBe(0);
  });

  test("refuses to start on a surface its Journey does not list", async ({ startRun }) => {
    await expect(startRun("J-961.toml", "cli")).rejects.toMatchObject({
      code: "surface_unlisted",
    });
  });
});
