// The smoke-script runner (ADR 0069): each `*.mjs` script of a folder, such
// as `sdks/typescript/smoke/`, is spawned by one Vitest test per runtime of
// the job's set and per mode, which checks its exit code and records the
// Client conformance cases the script declares, so the case-log reporter
// writes one entry per script, runtime and mode.
//
// A script declares its cases on a `// cases: <id>, <id>` line. It runs with
// the test's environment map (`env`), with `SMOKE_MODE` set to `attached` or
// `daemon`; in the `daemon` mode the test has started its Product's Daemon in
// the test's home first. The mode's variable is outside the `GOODFETCH_`
// prefix, whose unknown names an SDK refuses (ADR 0083).
//
// A script that reaches a remote Daemon over WebSocket declares it on a
// `// remote daemon: <corpus script> <token>` line, and runs in the `daemon`
// mode only: the test starts the Scripted engine as an Exposed Daemon playing
// that script of the shared corpus, gives the script `GOODFETCH_URL` and
// `GOODFETCH_TOKEN` naming it, and checks that the Daemon played its script
// through once the script exited.
//
// The runtimes are the job's set of `ci/runtimes.toml`, named by
// `GOODFETCH_RUNTIME_SET` (host-tools sets it; `pre-merge` otherwise): the
// Node that runs Vitest, which cell.sh runs once per Node line, and Bun and
// Deno from `PATH` when the set holds them. The edge runtimes, `workerd` and
// Vercel's edge runtime, run the scripts against a Daemon only, once #1421
// gives them a runner and an installer.
import { accessSync, constants, readdirSync, readFileSync } from "node:fs";
import { delimiter, join } from "node:path";
import { parse } from "smol-toml";
import { describe, expect } from "vitest";
import { CLIENT_CASE_TAG, hostRuntime } from "./case-log.ts";
import { TestSupportError } from "./errors.ts";
import { test } from "./fixtures.ts";
import type { Exit, ProcessRecord } from "./process-record.ts";
import type { Product } from "./product-env.ts";
import { startScriptedDaemon } from "./scripted-daemon.ts";

/** The remote Daemon a smoke script reaches: the Scripted engine playing a corpus script. */
export interface SmokeRemoteDaemon {
  /** The corpus script it plays, such as `connect/remote-drop-resends-a-read.json`. */
  readonly script: string;
  /** The token the script's Client presents, which the corpus script's hello holds. */
  readonly token: string;
}

/** A smoke script and the Client conformance cases it declares. */
export interface SmokeScript {
  /** Its file name, such as `hello.mjs`. */
  readonly name: string;
  readonly path: string;
  /** The IDs of its `// cases:` line, in their order. */
  readonly cases: readonly string[];
  /** The remote Daemon of its `// remote daemon:` line; a script without one runs attached and against a local Daemon. */
  readonly remoteDaemon: SmokeRemoteDaemon | undefined;
}

const CASES_LINE = /^\/\/ cases:(.*)$/m;
const REMOTE_DAEMON_LINE = /^\/\/ remote daemon:(.*)$/m;

/**
 * The `*.mjs` scripts of `folder`, by name; `smoke_cases_missing` for one
 * with no `// cases:` line naming a case, and `smoke_remote_daemon_invalid`
 * for a `// remote daemon:` line that names other than a corpus script and a
 * token.
 */
export function readSmokeScripts(folder: string): SmokeScript[] {
  return readdirSync(folder)
    .filter((name) => name.endsWith(".mjs"))
    .toSorted()
    .map((name) => {
      const path = join(folder, name);
      const text = readFileSync(path, "utf8");
      const cases = (CASES_LINE.exec(text)?.[1] ?? "")
        .split(",")
        .map((id) => id.trim())
        .filter((id) => id !== "");
      if (cases.length === 0) {
        throw new TestSupportError(
          "smoke_cases_missing",
          `the smoke script ${path} declares no Client conformance case: add a line \`// cases: <id>, <id>\``,
        );
      }
      return { name, path, cases, remoteDaemon: remoteDaemonOf(path, text) };
    });
}

/** The remote Daemon `text`'s `// remote daemon:` line names, if it has one. */
function remoteDaemonOf(path: string, text: string): SmokeRemoteDaemon | undefined {
  const line = REMOTE_DAEMON_LINE.exec(text)?.[1];
  if (line === undefined) {
    return undefined;
  }
  const words = line.trim().split(/\s+/);
  const [script, token] = words;
  if (words.length !== 2 || script === undefined || token === undefined) {
    throw new TestSupportError(
      "smoke_remote_daemon_invalid",
      `the smoke script ${path} has a remote-Daemon line that names other than a corpus script and a token: write \`// remote daemon: <corpus script> <token>\``,
    );
  }
  return { script, token };
}

/** A runtime a smoke script runs on: its ID in the case log, and the program that runs a script. */
export interface SmokeRuntime {
  /** Such as `node-26` or `bun-1`. */
  readonly id: string;
  readonly command: string;
  /** The program's arguments before the script's path. */
  readonly args: readonly string[];
}

/** The runtime matrix (ADR 0069), which `xtask runtimes` holds to the ADR's placement. */
const MATRIX = new URL("../../../ci/runtimes.toml", import.meta.url);

/** Where a set's runtimes are found. */
export interface SmokeRuntimeOptions {
  /** The set of `ci/runtimes.toml`, such as `release-linux`. */
  readonly set: string;
  /** The folders Bun and Deno are looked for in: `PATH`'s. */
  readonly path?: string | undefined;
  /** The runtime matrix; the repository's when absent. */
  readonly matrix?: string | URL | undefined;
}

/**
 * The runtimes the smoke scripts run on for `set`: the running Node, then
 * Bun and Deno when the set holds them. `runtime_set_unknown` when the
 * matrix holds no such set, and `runtime_missing` when the set's Bun or Deno
 * is not on the path.
 */
export function smokeRuntimes(options: SmokeRuntimeOptions): SmokeRuntime[] {
  const ids = setIds(options.matrix ?? MATRIX, options.set);
  const node: SmokeRuntime = { id: hostRuntime(), command: process.execPath, args: [] };
  const others = ids.flatMap((id): SmokeRuntime[] => {
    if (id.startsWith("bun-")) {
      return [{ id, command: onPath("bun", options.path, options.set), args: ["run"] }];
    }
    if (id.startsWith("deno-")) {
      return [
        { id, command: onPath("deno", options.path, options.set), args: ["run", "--allow-all"] },
      ];
    }
    return [];
  });
  return [node, ...others];
}

/** The runtime IDs of the matrix's set `set`. */
function setIds(matrix: string | URL, set: string): string[] {
  let sets: unknown;
  try {
    sets = parse(readFileSync(matrix, "utf8"))["sets"];
  } catch (error) {
    throw new TestSupportError(
      "runtime_set_unknown",
      `the runtime matrix ${String(matrix)} cannot be read`,
      { cause: error },
    );
  }
  const ids = isRecord(sets) ? sets[set] : undefined;
  if (!Array.isArray(ids) || !ids.every((id) => typeof id === "string")) {
    throw new TestSupportError(
      "runtime_set_unknown",
      `the runtime matrix ${String(matrix)} holds no set ${set}`,
    );
  }
  return ids;
}

/** The executable `name` in the first folder of `path` that holds one. */
function onPath(name: string, path: string | undefined, set: string): string {
  const extensions = process.platform === "win32" ? [".exe", ".cmd", ""] : [""];
  for (const folder of (path ?? "").split(delimiter).filter((entry) => entry !== "")) {
    for (const extension of extensions) {
      const candidate = join(folder, `${name}${extension}`);
      try {
        accessSync(candidate, constants.X_OK);
        return candidate;
      } catch {
        // Not in this folder; the next one may hold it.
      }
    }
  }
  throw new TestSupportError(
    "runtime_missing",
    `the runtime set ${set} holds ${name}, which is not on PATH: .github/actions/host-tools installs it`,
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** What the smoke tests run. */
export interface SmokeTestOptions {
  /** The folder of the `*.mjs` scripts. */
  readonly folder: string;
  /** The Product whose engine and Daemon the scripts reach: Collie's unless they need Foxhound. */
  readonly product?: Product | undefined;
}

/** The modes a smoke script runs in. */
type SmokeMode = "attached" | "daemon";

/** How long a remote-Daemon script's Daemon may take to end its play once the script exited. */
const DAEMON_EXIT_DEADLINE_MS = 5_000;

/**
 * Declares one test per script of `options.folder`, runtime of the job's set
 * and mode, tagged with the script's cases and the mode. Each test spawns
 * its script and checks that it exits 0, and for a remote-Daemon script,
 * which runs in the `daemon` mode only, that its Daemon played its script
 * through. A test file calls it inside a `describe` of its own: Vitest's
 * listing parses a test file without running it, and fails one that
 * declares no suite or test itself.
 */
export function smokeTests(options: SmokeTestOptions): void {
  const scripts = readSmokeScripts(options.folder);
  const runtimes = smokeRuntimes({
    set: process.env["GOODFETCH_RUNTIME_SET"] ?? "pre-merge",
    path: process.env["PATH"],
  });
  const product = options.product ?? "collie";
  const local = scripts.filter(({ remoteDaemon }) => remoteDaemon === undefined);
  for (const mode of local.length > 0 ? (["attached", "daemon"] as const) : []) {
    describe(`smoke scripts, ${mode}`, () => {
      test.override({ product, withDaemon: mode === "daemon" });
      for (const script of local) {
        for (const runtime of runtimes) {
          const tags = [...script.cases.map((id) => `${CLIENT_CASE_TAG}${id}`), mode];
          const name = `${script.name} on ${runtime.id}`;
          if (mode === "daemon") {
            test(name, { tags }, async ({ task, processes, env, daemon, clientCase }) => {
              expect(daemon.token).not.toBe("");
              task.meta.runtime = runtime.id;
              await clientCase.check(`the script exits 0 on ${runtime.id}`, async () => {
                await expectExitZero(processes, runtime, script, env, mode);
              });
            });
          } else {
            test(name, { tags }, async ({ task, processes, env, clientCase }) => {
              task.meta.runtime = runtime.id;
              await clientCase.check(`the script exits 0 on ${runtime.id}`, async () => {
                await expectExitZero(processes, runtime, script, env, mode);
              });
            });
          }
        }
      }
    });
  }
  const remotes = scripts.flatMap((script) =>
    script.remoteDaemon === undefined ? [] : [{ script, remote: script.remoteDaemon }],
  );
  if (remotes.length === 0) {
    return;
  }
  describe("smoke scripts, remote daemon", () => {
    test.override({ product, withDaemon: false });
    for (const { script, remote } of remotes) {
      for (const runtime of runtimes) {
        const tags = [...script.cases.map((id) => `${CLIENT_CASE_TAG}${id}`), "daemon"];
        test(
          `${script.name} on ${runtime.id}`,
          { tags },
          async ({ task, processes, testEnv, env, clientCase }) => {
            task.meta.runtime = runtime.id;
            const daemon = await startScriptedDaemon(
              testEnv,
              processes,
              remote.script,
              "websocket",
            );
            try {
              const reaching = {
                ...env,
                GOODFETCH_URL: `ws://${daemon.address}`,
                GOODFETCH_TOKEN: remote.token,
              };
              await clientCase.check(`the script exits 0 on ${runtime.id}`, async () => {
                await expectExitZero(processes, runtime, script, reaching, "daemon");
              });
              const exit = await exitWithin(daemon.exit, DAEMON_EXIT_DEADLINE_MS);
              await clientCase.check("the Scripted Daemon played its script through", () => {
                expect(exit, `the Scripted Daemon playing ${remote.script}`).toMatchObject({
                  code: 0,
                });
              });
            } finally {
              await daemon.kill();
            }
          },
        );
      }
    }
  });
}

/** How `exit` settled within `deadline` milliseconds, or `still running` when it had not. */
async function exitWithin(exit: Promise<Exit>, deadline: number): Promise<Exit | "still running"> {
  let timer: NodeJS.Timeout | undefined;
  const late = new Promise<"still running">((resolve) => {
    timer = setTimeout(() => resolve("still running"), deadline);
  });
  try {
    return await Promise.race([exit, late]);
  } finally {
    clearTimeout(timer);
  }
}

/** Spawns `script` on `runtime` with the test's map and the mode, and fails, with its output, unless it exits 0. */
async function expectExitZero(
  processes: ProcessRecord,
  runtime: SmokeRuntime,
  script: SmokeScript,
  env: Readonly<Record<string, string>>,
  mode: SmokeMode,
): Promise<void> {
  const run = processes.spawn(runtime.command, [...runtime.args, script.path], {
    env: { ...env, SMOKE_MODE: mode, DENO_NO_UPDATE_CHECK: "1", NO_COLOR: "1" },
  });
  run.child.stdin.end();
  let output = "";
  run.child.stdout.setEncoding("utf8").on("data", (chunk: string) => (output += chunk));
  run.child.stderr.setEncoding("utf8").on("data", (chunk: string) => (output += chunk));
  const exit = await run.exit;
  expect(
    { code: exit.code, signal: exit.signal, error: exit.error?.message },
    `${script.name} on ${runtime.id}, ${mode}:\n${output}`,
  ).toEqual({ code: 0, signal: null, error: undefined });
}
