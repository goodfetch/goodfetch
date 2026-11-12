// One test's setup from `goodfetch-test-env` (ADR 0062): TypeScript reuses
// the Rust setup and rewrites none of it. The executable, found through the
// test tool manifest, starts the Proving ground and the Daemon a test asks
// for, prints one JSON description, which is validated
// against the committed schema, and tears the setup down when its stdin
// closes.
import { readFileSync, writeFileSync } from "node:fs";
import type { Readable } from "node:stream";
import { setTimeout as delay } from "node:timers/promises";
import type { ValidateFunction } from "ajv/dist/2020.js";
import { TestSupportError } from "./errors.ts";
import type { ProcessRecord, RecordedProcess } from "./process-record.ts";
import type { DaemonDescription } from "./daemon.ts";
import type { Product } from "./product-env.ts";
import type { ProvingGroundDescription } from "./proving-ground.ts";
import { ajv, committedSchema } from "./schemas.ts";

/** The commit's executables, each present when its crate exists in the workspace. */
export interface Tools {
  /** `goodfetch-collie`, Collie's executable. */
  readonly collie?: string | undefined;
  /** `goodfetch-foxhound`, Foxhound's executable. */
  readonly foxhound?: string | undefined;
  /** The Proving ground's executable. */
  readonly proving_ground?: string | undefined;
  /** `goodfetch-test-env`. */
  readonly test_env?: string | undefined;
  /** `goodfetch-scripted-engine`, the Scripted engine. */
  readonly scripted_engine?: string | undefined;
  /** `goodfetch-conformance`, whose `read` loads a result document through `goodfetch-protocol`'s reader. */
  readonly conformance?: string | undefined;
  /** `goodfetch-stub-gateway`, the Stub gateway (ADR 0096). */
  readonly stub_gateway?: string | undefined;
}

/** One test's setup, as `goodfetch-test-env` describes it. */
export interface TestEnvDescription {
  /** The test's own folder, whose name holds a space and a non-ASCII letter. */
  readonly folder: string;
  /** `<folder>/home`, the `GOODFETCH_HOME` of everything the test starts. */
  readonly home: string;
  /** The cleared environment of every child the test starts. */
  readonly env: Readonly<Record<string, string>>;
  /** The test's seed. */
  readonly seed: number;
  /** The test's start time, RFC 3339 in UTC. */
  readonly start_time: string;
  /** The commit's executables, from the test tool manifest. */
  readonly tools: Tools;
  /** The Proving ground started on the seed the test asked for, if any. */
  readonly proving_ground?: ProvingGroundDescription | undefined;
  /** The Daemon started in the test's home, if the test asked for one. */
  readonly daemon?: DaemonDescription | undefined;
}

/** A program and its arguments. */
export interface Program {
  readonly command: string;
  readonly args: readonly string[];
}

/** How to start one test's setup. */
export interface TestEnvStart {
  /** The record the setup's process joins. */
  readonly processes: ProcessRecord;
  /** The test's full name, from which its seed and folder name come. */
  readonly testName: string;
  /** The test tool manifest's path: `GOODFETCH_TEST_TOOLS`, which `just test` sets. */
  readonly toolManifest: string | undefined;
  /** What runs in place of the manifest's `goodfetch-test-env`, when given. */
  readonly program?: Program | undefined;
  /** The seed of a Proving ground to start beside the setup, when given. */
  readonly provingGroundSeed?: number | undefined;
  /** The Product whose Daemon to start in the test's home, when given. */
  readonly daemon?: Product | undefined;
  /**
   * Whether the home is prepared for an engine that launches a browser:
   * its Browser cache laid out, and the Proving ground's hosts, when one
   * starts, reachable through its user configuration.
   */
  readonly browser?: boolean | undefined;
  /**
   * How long, in milliseconds, `goodfetch-test-env` has to tear down once
   * its stdin closes; ten seconds when absent. A test of a teardown past the
   * deadline sets a short one, so it waits no real ten seconds.
   */
  readonly teardownDeadlineMs?: number | undefined;
}

/** A started setup. */
export interface StartedTestEnv {
  readonly description: TestEnvDescription;
  /** The process ID of the program that described it. */
  readonly pid: number;
  /**
   * Closes `goodfetch-test-env`'s stdin and waits for it to tear the setup
   * down, keeping the test's folder when `failed` is set. Fails with
   * `teardown_failed` when it exits with a failure; one still running after
   * the deadline is left to the process record.
   */
  close(outcome?: { readonly failed: boolean }): Promise<void>;
}

/** How long `goodfetch-test-env` has to tear down once its stdin closes, unless the start says otherwise. */
const TEARDOWN_DEADLINE_MS = 10_000;

const TOOLS_VARIABLE = "GOODFETCH_TEST_TOOLS";

const RUN_WITH_JUST =
  "run the tests with `just test`, which builds the commit's executables and writes the test tool manifest";

const isTools: ValidateFunction<Tools> = ajv.compile<Tools>(
  committedSchema("test-tool-manifest.schema.json"),
);
const isDescription: ValidateFunction<TestEnvDescription> = ajv.compile<TestEnvDescription>(
  committedSchema("test-env-description.schema.json"),
);

/** Starts `goodfetch-test-env` for one test and reads its description. */
export async function startTestEnv(start: TestEnvStart): Promise<StartedTestEnv> {
  const program = start.program ?? {
    command: toolPath(readTools(start.toolManifest), "test_env"),
    args: [],
  };
  const env: Record<string, string | undefined> = {
    ...process.env,
    [TOOLS_VARIABLE]: start.toolManifest,
  };
  const provingGround =
    start.provingGroundSeed === undefined
      ? []
      : ["--proving-ground", String(start.provingGroundSeed)];
  const daemon = start.daemon === undefined ? [] : [`--daemon=${start.daemon}`];
  const browser = start.browser === true ? ["--browser"] : [];
  const started = start.processes.spawn(
    program.command,
    [...program.args, "--test-name", start.testName, ...provingGround, ...daemon, ...browser],
    {
      env,
    },
  );
  // Closing stdin of a process that already ended fails with EPIPE; its exit says how it ended.
  started.child.stdin.on("error", () => {});
  const stderr = forwardStderr(started.child.stderr);
  const line = await firstLine(started.child.stdout);
  if (line === undefined) {
    const exit = await started.exit;
    throw new TestSupportError(
      "start_failed",
      `goodfetch-test-env exited (code ${exit.code}, signal ${exit.signal}) before describing the test's setup: ${exit.error?.message ?? stderr().trim()}`,
      exit.error === undefined ? undefined : { cause: exit.error },
    );
  }
  const close = (outcome?: { readonly failed: boolean }): Promise<void> =>
    teardown(started, start.teardownDeadlineMs ?? TEARDOWN_DEADLINE_MS, outcome?.failed ?? false);
  const description = parseDescription(line);
  if (description === undefined) {
    await close();
    throw new TestSupportError(
      "description_invalid",
      `goodfetch-test-env printed a description that fails test-env-description.schema.json: ${ajv.errorsText(isDescription.errors)}`,
    );
  }
  return { description, pid: started.child.pid ?? 0, close };
}

/** The path of one executable of the manifest; `tool_missing` when it lists none. */
export function toolPath(tools: Tools, tool: keyof Tools): string {
  const path = tools[tool];
  if (path === undefined) {
    throw new TestSupportError(
      "tool_missing",
      `the test tool manifest lists no ${tool}: run the tests with \`just test\`, which builds every executable the workspace has`,
    );
  }
  return path;
}

/**
 * Writes at `path` the test tool manifest `manifest` with `engine` as the
 * executable of `product`, so everything the setup starts runs it, as a
 * skew run does (ADR 0066); gives `path`.
 */
export function manifestWithEngine(
  manifest: string | undefined,
  product: Product,
  engine: string,
  path: string,
): string {
  writeFileSync(path, `${JSON.stringify({ ...readTools(manifest), [product]: engine })}\n`);
  return path;
}

/** The test tool manifest at `manifest`; `tools_unset` or `tools_invalid` when it cannot be read. */
export function readTools(manifest: string | undefined): Tools {
  if (manifest === undefined) {
    throw new TestSupportError("tools_unset", `${TOOLS_VARIABLE} is not set: ${RUN_WITH_JUST}`);
  }
  let tools: unknown;
  try {
    tools = JSON.parse(readFileSync(manifest, "utf8"));
  } catch (cause) {
    throw new TestSupportError(
      "tools_invalid",
      `the test tool manifest ${manifest} cannot be read: ${RUN_WITH_JUST}`,
      {
        cause,
      },
    );
  }
  if (!isTools(tools)) {
    throw new TestSupportError(
      "tools_invalid",
      `the test tool manifest ${manifest} fails test-tool-manifest.schema.json (${ajv.errorsText(isTools.errors)}): ${RUN_WITH_JUST}`,
    );
  }
  return tools;
}

function parseDescription(line: string): TestEnvDescription | undefined {
  let description: unknown;
  try {
    description = JSON.parse(line);
  } catch {
    return undefined;
  }
  return isDescription(description) ? description : undefined;
}

async function teardown(
  started: RecordedProcess,
  deadlineMs: number,
  failed: boolean,
): Promise<void> {
  // A failed test's folder stays for its evidence, as a Rust test's does.
  started.child.stdin.end(failed ? "failed\n" : undefined);
  const deadline = new AbortController();
  const exit = await Promise.race([
    started.exit,
    delay(deadlineMs, undefined, { signal: deadline.signal }).catch(() => undefined),
  ]);
  deadline.abort();
  if (exit !== undefined && exit.code !== 0) {
    throw new TestSupportError(
      "teardown_failed",
      `goodfetch-test-env failed to tear the test's setup down (code ${exit.code}, signal ${exit.signal})`,
    );
  }
}

/** Reads a stream's first line; `undefined` when it ends without one. */
function firstLine(stream: Readable): Promise<string | undefined> {
  stream.setEncoding("utf8");
  return new Promise((resolve) => {
    let text = "";
    const onData = (chunk: string): void => {
      text += chunk;
      const end = text.indexOf("\n");
      if (end !== -1) {
        stream.off("data", onData);
        stream.off("end", onEnd);
        // Whatever follows is drained, so the stream can close.
        stream.resume();
        resolve(text.slice(0, end));
      }
    };
    const onEnd = (): void => {
      resolve(undefined);
    };
    stream.on("data", onData);
    stream.once("end", onEnd);
  });
}

/**
 * Forwards `goodfetch-test-env`'s stderr, its seed line among it, to the
 * test's own output, and keeps it for a failed start's message.
 */
function forwardStderr(stream: Readable): () => string {
  let text = "";
  let forwarded = 0;
  stream.setEncoding("utf8");
  stream.on("data", (chunk: string) => {
    text += chunk;
    const end = text.lastIndexOf("\n") + 1;
    if (end > forwarded) {
      console.error(text.slice(forwarded, end).trimEnd());
      forwarded = end;
    }
  });
  return () => text;
}
