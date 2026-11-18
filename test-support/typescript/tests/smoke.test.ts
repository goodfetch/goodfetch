// Seam B of #412: the smoke-script runner (ADR 0069). A fixture run of a
// passing and a failing script, on a set holding Bun and Deno, writes one
// case-log entry per script, runtime and mode, and each entry's check says
// whether the script exited 0. Bun and Deno are stand-ins on `PATH` that run
// the script on this Node, so the run needs neither installed.
import { chmodSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { delimiter, dirname, join } from "node:path";
import { describe, expect } from "vitest";
import { test, type ProcessRecord } from "@goodfetch/test-support";
import { hostRuntime } from "@goodfetch/test-support/case-log";
import { readSmokeScripts, smokeRuntimes } from "@goodfetch/test-support/smoke";

const FIXTURE = join(import.meta.dirname, "fixtures", "smoke");
const VITEST = join(
  dirname(createRequire(import.meta.url).resolve("vitest/package.json")),
  "vitest.mjs",
);

/** A folder holding `bun` and `deno` stand-ins that run a script on this Node, as `bun run` and `deno run --allow-all` would. */
function standIns(folder: string): string {
  const bin = join(folder, "bin");
  mkdirSync(bin, { recursive: true });
  for (const [name, dropped] of [
    ["bun", 1],
    ["deno", 2],
  ] as const) {
    const path = join(bin, name);
    writeFileSync(path, `#!/bin/sh\nshift ${dropped}\nexec '${process.execPath}' "$@"\n`);
    chmodSync(path, 0o755);
  }
  return bin;
}

/** Runs the fixture's smoke tests on `set` with `bin` first on `PATH`, the reporter writing to `log`, and gives Vitest's exit code. */
async function runSmoke(
  processes: ProcessRecord,
  set: string,
  bin: string,
  log: string,
): Promise<number | null> {
  const run = processes.spawn(
    process.execPath,
    [VITEST, "run", "--config", join(FIXTURE, "vitest.config.ts")],
    {
      env: {
        ...process.env,
        GOODFETCH_CASE_LOG: log,
        GOODFETCH_RUNTIME_SET: set,
        PATH: `${bin}${delimiter}${process.env["PATH"] ?? ""}`,
      },
      cwd: FIXTURE,
    },
  );
  run.child.stdin.end();
  run.child.stdout.resume();
  run.child.stderr.resume();
  return (await run.exit).code;
}

/** Each entry of the case log at `log` as its case, runtime, mode and checks, sorted. */
function entries(log: string): string[] {
  return readFileSync(log, "utf8")
    .split("\n")
    .filter((line) => line !== "")
    .map((line) => {
      const entry: { case: string; runtime: string; mode: string; checks: unknown[] } =
        JSON.parse(line);
      return `${entry.case} ${entry.runtime} ${entry.mode} ${JSON.stringify(entry.checks)}`;
    })
    .toSorted();
}

/** The checks of a passing or failing script's entry on `runtime`. */
function checks(runtime: string, passed: boolean): string {
  return JSON.stringify([
    { name: `the script exits 0 on ${runtime}`, passed },
    { name: "the test passed", passed },
  ]);
}

// The stand-ins are POSIX shell scripts, which Windows cannot spawn.
describe.skipIf(process.platform === "win32")("the smoke-script runner's run", () => {
  test("writes a case-log entry per script, runtime and mode, each checking the script's exit", async ({
    processes,
    testEnv,
  }) => {
    const log = join(testEnv.folder, "case-log.jsonl");
    const bin = standIns(testEnv.folder);

    const exit = await runSmoke(processes, "release-other", bin, log);

    expect(exit).toBe(1);
    const node = hostRuntime();
    expect(entries(log)).toEqual(
      [
        ["cancel-mid-run", false],
        ["hello-refused", true],
      ]
        .flatMap(([id, passed]) =>
          [node, "bun-1", "deno-2"].flatMap((runtime) =>
            ["attached", "daemon"].map(
              (mode) => `${String(id)} ${runtime} ${mode} ${checks(runtime, passed === true)}`,
            ),
          ),
        )
        .toSorted(),
    );
  });
});

describe("the smoke-script runner", () => {
  test("reads each script's declared cases", () => {
    const scripts = readSmokeScripts(join(FIXTURE, "scripts"));

    expect(scripts.map(({ name, cases }) => ({ name, cases }))).toEqual([
      { name: "fails.mjs", cases: ["cancel-mid-run"] },
      { name: "passes.mjs", cases: ["hello-refused"] },
    ]);
  });

  test("refuses a script that declares no case", () => {
    expect(() =>
      readSmokeScripts(join(import.meta.dirname, "fixtures", "smoke-headerless")),
    ).toThrow(expect.objectContaining({ code: "smoke_cases_missing" }));
  });

  test("runs the set's Bun and Deno from PATH, and the Node running the tests", ({ testEnv }) => {
    const bin = standIns(testEnv.folder);

    const runtimes = smokeRuntimes({ set: "release-linux", path: bin });

    expect(runtimes).toEqual([
      { id: hostRuntime(), command: process.execPath, args: [] },
      { id: "bun-1", command: join(bin, "bun"), args: ["run"] },
      { id: "deno-2", command: join(bin, "deno"), args: ["run", "--allow-all"] },
    ]);
    expect(smokeRuntimes({ set: "pre-merge", path: bin }).map(({ id }) => id)).toEqual([
      hostRuntime(),
    ]);
  });

  test("refuses a set whose Bun is not on PATH", () => {
    expect(() => smokeRuntimes({ set: "release-other", path: "" })).toThrow(
      expect.objectContaining({ code: "runtime_missing" }),
    );
  });

  test("refuses a set the runtime matrix does not hold", () => {
    expect(() => smokeRuntimes({ set: "ends", path: "" })).toThrow(
      expect.objectContaining({ code: "runtime_set_unknown" }),
    );
  });
});
