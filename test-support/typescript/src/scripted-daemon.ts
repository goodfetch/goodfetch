// The Scripted engine played as a Daemon (ADR 0069), for an SDK's tests of
// how `connect()` finds one: `socket` publishes it in the runtime file of a
// Goodfetch home, as a running Daemon is found, and `tcp` listens on
// loopback TCP, as an Exposed Daemon `GOODFETCH_URL` names does. It plays
// one script of the shared corpus to one Client and exits, 0 once every
// step played, so a test that awaits its exit learns whether the Client
// sent exactly what the script holds.
import { fileURLToPath } from "node:url";
import { TestSupportError } from "./errors.ts";
import type { Exit, ProcessRecord } from "./process-record.ts";
import { corpusScript } from "./product-env.ts";
import { toolPath, type TestEnvDescription } from "./test-env.ts";

/** How a Client reaches the Scripted Daemon. */
export type ScriptedDaemonMode = "socket" | "tcp";

/** A Scripted engine waiting for its Client. */
export interface ScriptedDaemon {
  /** Where it listens: a Unix socket's path, or `host:port` on loopback TCP. */
  readonly address: string;
  /** Settles once its play ended and it exited. */
  readonly exit: Promise<Exit>;
  /** Kills it, for a test whose Client never reaches it, and settles once it exited. */
  kill(): Promise<Exit>;
}

/**
 * Starts `goodfetch-scripted-engine` playing the corpus's script `script` in
 * `mode`, recorded in `processes`, and resolves once a Client can reach it:
 * in `socket` mode its runtime file is in `home`'s per-major folder.
 * `tool_missing` when the manifest lists no Scripted engine, and
 * `scripted_daemon_failed`, once it is killed, when it exits or prints
 * anything else before saying where it listens.
 */
export async function startScriptedDaemon(
  description: TestEnvDescription,
  processes: ProcessRecord,
  script: string,
  mode: ScriptedDaemonMode,
  home: string = description.home,
): Promise<ScriptedDaemon> {
  const engine = toolPath(description.tools, "scripted_engine");
  const started = processes.spawn(
    engine,
    ["--script", fileURLToPath(corpusScript(script)), "--mode", mode],
    { env: { ...description.env, GOODFETCH_HOME: home } },
  );
  const { stdout, stderr } = started.child;
  stdout.setEncoding("utf8");
  stderr.setEncoding("utf8");
  let failure = "";
  stderr.on("data", (text: string) => {
    failure += text;
  });
  const address = await new Promise<string | undefined>((resolve) => {
    let line = "";
    const read = (text: string): void => {
      line += text;
      const end = line.indexOf("\n");
      if (end !== -1) {
        stdout.off("data", read);
        stdout.resume();
        resolve(addressOf(line.slice(0, end)));
      }
    };
    stdout.on("data", read);
    void started.exit.then(() => resolve(undefined));
  });
  if (address === undefined) {
    // It exited, or said something else than where it listens: either way
    // no Client reaches it.
    started.child.kill("SIGKILL");
    const exit = await started.exit;
    throw new TestSupportError(
      "scripted_daemon_failed",
      `the Scripted engine playing ${script} in ${mode} mode ended (${exit.signal ?? `code ${exit.code}`}) without saying where it listens: ${failure.trim()}`,
    );
  }
  return {
    address,
    exit: started.exit,
    kill: () => {
      started.child.kill("SIGKILL");
      return started.exit;
    },
  };
}

/** The `address` of the line the Scripted engine prints once it listens; `undefined` for any other line. */
function addressOf(line: string): string | undefined {
  let announced: unknown;
  try {
    announced = JSON.parse(line);
  } catch {
    return undefined;
  }
  return typeof announced === "object" &&
    announced !== null &&
    "address" in announced &&
    typeof announced.address === "string"
    ? announced.address
    : undefined;
}
