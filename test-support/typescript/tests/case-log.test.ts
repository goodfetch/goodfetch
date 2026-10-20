// Seam C (spec #326): the case log the Vitest reporter writes for Client
// conformance cases (ADR 0069). A fixture run of a passing, a failing and a
// skipped case writes entries that match the bundle's `case_log_entry`
// schema and set no outcome; `goodfetch-conformance write` turns them into a
// `conformance` document, and `goodfetch-conformance read` accepts it.
import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { Ajv2020 } from "ajv/dist/2020.js";
import { describe, expect } from "vitest";
import {
  test,
  toolPath,
  type ProcessRecord,
  type TestEnvDescription,
} from "@goodfetch/test-support";
import { hostPlatform, hostRuntime } from "@goodfetch/test-support/case-log";

const FIXTURE = join(import.meta.dirname, "fixtures", "case-log");
const VITEST = join(
  dirname(createRequire(import.meta.url).resolve("vitest/package.json")),
  "vitest.mjs",
);

const bundle: { $defs: object } = JSON.parse(
  readFileSync(
    new URL("../../../crates/goodfetch-protocol/generated/bundle.json", import.meta.url),
    "utf8",
  ),
);
const isCaseLogEntry = new Ajv2020({ allErrors: true }).compile({
  $ref: "#/$defs/CaseLogEntry",
  $defs: bundle.$defs,
});

/** Runs the fixture's cases with the reporter writing to `log`, and gives Vitest's exit code. */
async function runCases(processes: ProcessRecord, log: string): Promise<number | null> {
  const run = processes.spawn(
    process.execPath,
    [VITEST, "run", "--config", join(FIXTURE, "vitest.config.ts")],
    { env: { ...process.env, GOODFETCH_CASE_LOG: log }, cwd: FIXTURE },
  );
  run.child.stdin.end();
  run.child.stdout.resume();
  run.child.stderr.resume();
  return (await run.exit).code;
}

/** The entries of the case log at `log`, in the order of their cases' IDs. */
function entries(log: string): Record<string, unknown>[] {
  return readFileSync(log, "utf8")
    .split("\n")
    .filter((line) => line !== "")
    .map((line): Record<string, unknown> => JSON.parse(line))
    .toSorted((left, right) => String(left["case"]).localeCompare(String(right["case"])));
}

/** Runs `goodfetch-conformance` with `args`, and gives its exit code. */
async function conformance(
  processes: ProcessRecord,
  testEnv: TestEnvDescription,
  args: string[],
): Promise<number | null> {
  const run = processes.spawn(toolPath(testEnv.tools, "conformance"), args, { env: testEnv.env });
  run.child.stdin.end();
  run.child.stdout.resume();
  run.child.stderr.resume();
  return (await run.exit).code;
}

describe("the case-log reporter", () => {
  test("writes one entry per case run, with each check and no outcome", async ({
    processes,
    testEnv,
  }) => {
    const log = join(testEnv.folder, "case-log.jsonl");

    const exit = await runCases(processes, log);

    expect(exit).toBe(1);
    const written = entries(log);
    expect(written.map((entry) => isCaseLogEntry(entry))).toEqual([true, true]);
    expect(written).toMatchObject([
      {
        case: "cancel-mid-run",
        capability: "cancel",
        mode: "attached",
        checks: [
          { name: "the call sends $/cancel", passed: true },
          { name: "the run ends cancelled", passed: false },
          { name: "the test passed", passed: false },
        ],
        evidence: {},
      },
      {
        case: "hello-refused",
        capability: "hello",
        mode: "attached",
        checks: [
          { name: "the Client raises protocol_unsupported", passed: true },
          { name: "the test passed", passed: true },
        ],
        evidence: {
          messages: [
            { method: "hello", params: { protocol: "1.4" } },
            { error: { code: "protocol_unsupported" } },
          ],
        },
      },
    ]);
    expect(written.filter((entry) => "outcome" in entry)).toEqual([]);
  });

  test("writes a log that goodfetch-conformance turns into a document the reader accepts", async ({
    processes,
    testEnv,
  }) => {
    const log = join(testEnv.folder, "case-log.jsonl");
    const header = join(testEnv.folder, "header.json");
    const document = join(testEnv.folder, "conformance.json");
    await runCases(processes, log);
    writeFileSync(
      header,
      JSON.stringify({
        suite: { name: "clients", version: "1" },
        target: { name: "goodfetch-collie", version: "0.0.0" },
        platform: hostPlatform(),
        runtime: hostRuntime(),
        proving_ground: { version: "0.0.0", seed: testEnv.seed },
        online: false,
      }),
    );

    const written = await conformance(processes, testEnv, [
      "write",
      join(FIXTURE, "cases.toml"),
      header,
      log,
      document,
    ]);

    expect(written).toBe(0);
    expect(await conformance(processes, testEnv, ["read", document])).toBe(0);
    expect(JSON.parse(readFileSync(document, "utf8"))).toMatchObject({
      kind: "conformance",
      capabilities: [
        { name: "hello", outcome: "proved", cases: [{ id: "hello-refused", outcome: "passed" }] },
        { name: "cancel", outcome: "failed", cases: [{ id: "cancel-mid-run", outcome: "failed" }] },
        {
          name: "credits",
          outcome: "unproved",
          cases: [{ id: "credits-released", outcome: "not_reached" }],
        },
      ],
    });
  });
});
