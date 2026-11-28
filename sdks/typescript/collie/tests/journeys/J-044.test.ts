// J-044, "Run the same SDK program on an Attached engine, then an Exposed
// Daemon by two variables", on the `sdk-typescript` surface (ADRs 0049, 0063,
// 0067, 0078, 0083, 0092 and 0095; CL-06): the same TypeScript SDK program
// runs with `engine` left at `auto` first on an Attached engine and then on
// an Exposed Daemon by setting `GOODFETCH_URL` and `GOODFETCH_TOKEN`, with
// no code change. Rust's `tests/it/journeys/j044.rs` and Python's
// `tests/journeys/test_j044.py` are its twins.
//
// Covers: story:14-62, story:14-63, row:CL-06
import { Collie, type EngineInfo, type Logger } from "@goodfetch/collie";
import { journey, searchCanaries, taskStartUrl, test, type Canary } from "@goodfetch/test-support";
import { spawn, type ChildProcessByStdio } from "node:child_process";
import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { createInterface } from "node:readline";
import type { Readable } from "node:stream";
import { expect } from "vitest";

/** The Run ID the shared SDK program uses on both engines. */
const RUN_ID = "j044-same-program";

/** The setting the shared SDK program explains, unset in both homes. */
const EXPLAINED_KEY = "timeout.navigation";

/** The bytes stored and read back by the shared SDK program. */
const STORED_BYTES = new TextEncoder().encode("j044-same-program-bytes");

/** A visibly fake endpoint token canary planted into the Exposed Daemon (ADR 0067). */
const TOKEN_CANARY: Canary = {
  id: "j044-exposed-daemon-token",
  kind: "endpoint_token",
  value: "endpoint-token-canary-j044-000000000044",
};

/** One log entry captured from the SDK's host logger. */
interface Logged {
  readonly level: "debug" | "info" | "warn" | "error";
  readonly message: string;
  readonly fields: Readonly<Record<string, unknown>> | undefined;
}

/** The deterministic result of the shared SDK program, compared across engines. */
interface ProgramResult {
  readonly runId: string;
  readonly lifecycle: string;
  readonly absent: unknown;
  readonly stored: unknown;
  readonly storedBytes: {
    readonly content: readonly number[];
    readonly contentType: string;
  } | null;
  readonly deleted: readonly [boolean, boolean];
  readonly explained: unknown;
}

/** A host logger that keeps every SDK log record. */
function keepingLogger(): { readonly logger: Logger; readonly logged: Logged[] } {
  const logged: Logged[] = [];
  const keep =
    (level: Logged["level"]) =>
    (message: string, fields?: Readonly<Record<string, unknown>>): void => {
      logged.push({ level, message, fields });
    };
  return {
    logger: {
      level: "debug",
      debug: keep("debug"),
      info: keep("info"),
      warn: keep("warn"),
      error: keep("error"),
    },
    logged,
  };
}

/** The per-major folder name for `version`: `0.<minor>` before 1.0, `<major>` from 1.0 (ADR 0030). */
function versionLine(version: string): string {
  const [major, minor] = version.split(".");
  if (major === undefined || minor === undefined) {
    throw new TypeError(`unexpected Product version: ${version}`);
  }
  return major === "0" ? `0.${minor}` : major;
}

/** Reads the executable's `goodfetch-collie <version>` output. */
async function executableVersion(
  executable: string,
  env: Readonly<Record<string, string>>,
  cwd: string,
): Promise<string> {
  const child = spawn(executable, ["--version"], {
    env,
    cwd,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  let stdout = "";
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => {
    stdout += chunk;
  });
  const code = await new Promise<number | null>((resolve, reject) => {
    child.once("error", reject);
    child.once("close", (exitCode) => resolve(exitCode));
  });
  const match = /^goodfetch-collie (\S+)$/.exec(stdout.trim());
  if (code !== 0 || match?.[1] === undefined) {
    throw new Error(`could not read executable version (${String(code)}): ${stdout}`);
  }
  return match[1];
}

/** An Exposed Daemon started with `daemon run` in a separate Goodfetch home. */
interface ExposedDaemon {
  readonly url: string;
  readonly logged: string[];
  readonly child: ChildProcessByStdio<null, Readable, Readable>;
  readonly exit: Promise<number | null>;
}

async function startExposedDaemon(
  executable: string,
  baseEnv: Readonly<Record<string, string>>,
  folder: string,
  version: string,
  token: string,
): Promise<ExposedDaemon> {
  const daemonHome = join(folder, "daemon-home");
  const majorDir = join(daemonHome, versionLine(version));
  await mkdir(majorDir, { recursive: true });
  await writeFile(
    join(majorDir, "configuration.toml"),
    'version = "0.1"\n\n[settings]\ndaemon.address = "127.0.0.1:0"\n',
    {
      mode: 0o600,
    },
  );
  await writeFile(join(majorDir, "daemon.token"), token, { mode: 0o600 });

  const child = spawn(executable, ["daemon", "run"], {
    env: { ...baseEnv, GOODFETCH_HOME: daemonHome },
    cwd: folder,
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  child.stderr.resume();
  const logged: string[] = [];
  const exit = new Promise<number | null>((resolve) => {
    child.once("close", (code) => resolve(code));
  });

  const reader = createInterface({ input: child.stdout });
  const endpoint = await new Promise<string>((resolve, reject) => {
    const timer = setTimeout(() => {
      reject(new Error(`Exposed Daemon never served; logged:\n${logged.join("\n")}`));
    }, 60_000);
    let resolved = false;
    reader.on("line", (line) => {
      logged.push(line);
      if (resolved) {
        return;
      }
      try {
        const parsed: unknown = JSON.parse(line);
        if (typeof parsed === "object" && parsed !== null) {
          const fields: unknown = Reflect.get(parsed, "fields");
          if (
            typeof fields === "object" &&
            fields !== null &&
            Reflect.get(fields, "message") === "the foreground Daemon serves"
          ) {
            const bound: unknown = Reflect.get(fields, "endpoint");
            if (typeof bound === "string") {
              resolved = true;
              clearTimeout(timer);
              resolve(bound);
            }
          }
        }
      } catch {
        // Ignore non-JSON lines while scanning for the serve record.
      }
    });
    child.once("error", (error) => {
      if (!resolved) {
        clearTimeout(timer);
        reject(error);
      }
    });
    child.once("close", (code) => {
      if (!resolved) {
        clearTimeout(timer);
        reject(
          new Error(
            `Exposed Daemon exited (${String(code)}) before serving:\n${logged.join("\n")}`,
          ),
        );
      }
    });
  });

  return { url: `ws://${endpoint}`, logged, child, exit };
}

/**
 * The shared SDK program: leaves `engine` at `auto`, connects with `env`,
 * runs Collie operations, finishes the Run, and closes the Connection.
 */
async function runProgram(
  env: Readonly<Record<string, string>>,
  seedUrl: string,
  logger: Logger,
): Promise<{ readonly engine: EngineInfo; readonly result: ProgramResult }> {
  // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
  const collie = await Collie.connect({ env, logger });
  try {
    const engine = collie.engine();
    const started = await collie.start({ runId: RUN_ID });
    const absent = await started.keyValueStore.get("item");
    const payload = { seed_url: seedUrl, count: 2, nested: [1, null] };
    await started.keyValueStore.set("item", payload);
    const stored = await started.keyValueStore.get("item");
    await started.keyValueStore.setBytes("note", STORED_BYTES, "text/plain");
    const bytes = await started.keyValueStore.getBytes("note");
    const deleted: readonly [boolean, boolean] = [
      await started.keyValueStore.delete("item"),
      await started.keyValueStore.delete("item"),
    ];
    const explained = await started.explainSettings([EXPLAINED_KEY]);
    const report = await started.finish();
    return {
      engine,
      result: {
        runId: report.runId,
        lifecycle: report.lifecycle,
        absent,
        stored,
        storedBytes:
          bytes === null ? null : { content: [...bytes.content], contentType: bytes.contentType },
        deleted,
        explained: explained.settings,
      },
    };
  } finally {
    await collie.close();
  }
}

test(
  "J-044@sdk-typescript",
  { tags: ["J-044", "sdk-typescript", "story:14-62", "story:14-63", "row:CL-06"] },
  journey(async (run) => {
    const provingGround = run.provingGround;
    if (provingGround === undefined) {
      throw new TypeError("J-044 has a Proving ground");
    }
    const seedUrl = await taskStartUrl(provingGround, "catalog/2");
    const version = await executableVersion(run.engine, run.testEnv.env, run.testEnv.folder);

    let daemon: ExposedDaemon | undefined;
    let localOutcome: { readonly engine: EngineInfo; readonly result: ProgramResult } | undefined;
    let remoteOutcome: { readonly engine: EngineInfo; readonly result: ProgramResult } | undefined;
    let daemonStopped = false;
    const remoteLogger = keepingLogger();

    try {
      daemon = await run.step(
        "Start an Exposed Daemon in a separate Goodfetch home on loopback port 0 with a planted Secret canary as its persistent token",
        async () =>
          startExposedDaemon(
            run.engine,
            run.testEnv.env,
            run.testEnv.folder,
            version,
            TOKEN_CANARY.value,
          ),
      );
      localOutcome = await run.step(
        "Run the SDK program with `engine` left at `auto` and neither `GOODFETCH_URL` nor `GOODFETCH_TOKEN` set, reading seeded Proving ground input, storing and reading values in the Run's Key-value store, explaining settings, and finishing the Run",
        async () => runProgram(run.env, seedUrl, keepingLogger().logger),
      );
      remoteOutcome = await run.step(
        "Run the exact same SDK program with `GOODFETCH_URL` pointing at the Exposed Daemon and `GOODFETCH_TOKEN` carrying the Secret canary, with no code change",
        async () => {
          if (daemon === undefined) {
            throw new TypeError("the Exposed Daemon started");
          }
          return runProgram(
            { ...run.env, GOODFETCH_URL: daemon.url, GOODFETCH_TOKEN: TOKEN_CANARY.value },
            seedUrl,
            remoteLogger.logger,
          );
        },
      );
      await run.step("Stop the Exposed Daemon cleanly and collect its logs", async () => {
        if (daemon === undefined) {
          throw new TypeError("the Exposed Daemon started");
        }
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        const stopConn = await Collie.connect({
          env: { ...run.env, GOODFETCH_URL: daemon.url, GOODFETCH_TOKEN: TOKEN_CANARY.value },
        });
        try {
          await stopConn.call("daemon.stop", {});
        } finally {
          await stopConn.close();
        }
        const code = await daemon.exit;
        daemonStopped = code === 0;
      });

      await run.check(
        "The program returned identical results on the local engine and on the Exposed Daemon",
        () => {
          if (localOutcome === undefined || remoteOutcome === undefined) {
            throw new TypeError("both runs completed");
          }
          expect(localOutcome.engine.kind).toBe("attached");
          expect(remoteOutcome.result).toEqual(localOutcome.result);
          expect(daemonStopped).toBe(true);
        },
      );
      await run.check(
        "On the Exposed Daemon, `EngineInfo` reported `kind` as `remote`, the `url` reached, Product `collie`, the executable's Product version, and protocol version `0.1`, with no home, engine path, or fallback",
        () => {
          if (daemon === undefined || remoteOutcome === undefined) {
            throw new TypeError("the remote run completed");
          }
          expect(remoteOutcome.engine).toEqual({
            kind: "remote",
            product: "collie",
            version,
            protocolVersion: "0.1",
            projectFile: null,
            home: null,
            enginePath: null,
            url: daemon.url,
            fallback: null,
          });
        },
      );
      await run.check(
        "The Secret canary in `GOODFETCH_TOKEN` appeared nowhere in `EngineInfo`, SDK logs, Exposed Daemon logs, or program outputs",
        () => {
          if (daemon === undefined || remoteOutcome === undefined) {
            throw new TypeError("the remote run completed");
          }
          const hits = searchCanaries(
            [TOKEN_CANARY],
            [
              { name: "engine", text: JSON.stringify(remoteOutcome.engine) },
              { name: "sdk_logs", text: JSON.stringify(remoteLogger.logged) },
              { name: "daemon_logs", text: daemon.logged.join("\n") },
              { name: "program_output", text: JSON.stringify(remoteOutcome.result) },
            ],
          );
          expect(hits).toEqual([]);
        },
      );
    } finally {
      if (
        daemon !== undefined &&
        daemon.child.exitCode === null &&
        daemon.child.signalCode === null
      ) {
        daemon.child.kill("SIGKILL");
        await daemon.exit;
      }
    }
  }),
);
