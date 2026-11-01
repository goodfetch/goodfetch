// The Stub gateway as a served test's Console (ADR 0096; TESTING.md, The
// Console conformance suite): `goodfetch-stub-gateway`, from the test tool
// manifest, serves Foxhound's Console build beside the test's Daemon, its
// state in the test's folder, and runs each run on an Attached engine of the
// Product the routing rule picks, Collie's and Foxhound's executables taken
// from the same manifest. A person signs in with the link a gateway gives: its
// token in the address's fragment, which the page swaps at `POST /sign-in`
// for the gateway's cookie.
import { randomBytes } from "node:crypto";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import { TestSupportError } from "./errors.ts";
import type { ConsoleServer } from "./served-daemon.ts";
import { toolPath } from "./test-env.ts";

/** Foxhound's Console build, `xtask console build`'s output folder, which `just test` builds. */
export const FOXHOUND_CONSOLE_BUILD: string = join(
  import.meta.dirname,
  "..",
  "..",
  "..",
  "crates",
  "foxhound",
  "goodfetch-foxhound",
  "console-build",
);

/** How much of the gateway's log a failed start reports. */
const STDERR_KEPT = 16_384;

/** How long the gateway may take to write its ready file. */
const READY_WITHIN_MS = 30_000;

/** The gateway's origins, as its ready file names them. */
interface Ready {
  readonly console: string;
  readonly content: string;
}

/** The ready file's origins, once the gateway wrote it. */
async function readyOf(path: string): Promise<Ready | undefined> {
  const text = await readFile(path, "utf8").catch(() => undefined);
  if (text === undefined || text === "") {
    return undefined;
  }
  const ready: unknown = JSON.parse(text);
  return typeof ready === "object" &&
    ready !== null &&
    "console" in ready &&
    "content" in ready &&
    typeof ready.console === "string" &&
    typeof ready.content === "string"
    ? { console: ready.console, content: ready.content }
    : undefined;
}

/**
 * The Stub gateway beside the test's Daemon: its Console the gateway's,
 * each run on an engine of its own under `<folder>/stub-gateway/`.
 */
export const stubGatewayConsole: ConsoleServer = {
  serve: async (start) => {
    const state = join(dirname(start.home), "stub-gateway");
    await mkdir(state, { recursive: true });
    const token = randomBytes(32).toString("hex");
    const tokenFile = join(state, "token");
    const readyFile = join(state, "ready.json");
    await writeFile(tokenFile, token, { mode: 0o600 });
    const gateway = start.processes.spawn(
      toolPath(start.tools, "stub_gateway"),
      [
        "--console-build",
        FOXHOUND_CONSOLE_BUILD,
        "--collie",
        toolPath(start.tools, "collie"),
        "--foxhound",
        toolPath(start.tools, "foxhound"),
        "--token-file",
        tokenFile,
        "--state",
        join(state, "engines"),
        "--ready",
        readyFile,
      ],
      { env: start.env },
    );
    // Its last log lines, which say why it did not start.
    let stderr = "";
    gateway.child.stderr.on("data", (chunk: Buffer) => {
      stderr = `${stderr}${chunk.toString()}`.slice(-STDERR_KEPT);
    });
    let ended = false;
    gateway.child.once("exit", () => {
      ended = true;
    });
    const deadline = Date.now() + READY_WITHIN_MS;
    let ready = await readyOf(readyFile);
    while (ready === undefined) {
      if (ended || Date.now() > deadline) {
        gateway.child.kill("SIGKILL");
        await gateway.exit;
        throw new TestSupportError(
          "console_not_served",
          `the Stub gateway wrote no ready file: ${stderr}`,
        );
      }
      await delay(50);
      ready = await readyOf(readyFile);
    }
    const url = new URL("/", ready.console).href;
    return {
      url,
      signInLink: async () => `${url}#token=${token}`,
      singleSignOn: new URL("sso", url).href,
      session: new URL("session", url).href,
      gateway: { token, content: ready.content, engines: join(state, "engines") },
      close: async () => {
        // An interrupt stops it as Ctrl-C does: its engines end with it.
        gateway.child.kill("SIGINT");
        await gateway.exit;
      },
    };
  },
};
