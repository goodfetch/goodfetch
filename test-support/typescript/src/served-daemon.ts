// The Daemon of one served Console test (ADRs 0068 and 0094): a Daemon of
// its own, started by `goodfetch-test-env --daemon` in a Goodfetch home of
// its own and run by the executable of the Product whose Console build the
// test covers, with the Console it serves on a port of its own, and a
// Proving ground when the test asks for one. Tests then run in any order.
//
// The Console is served by a `ConsoleServer`, given the started Daemon and
// the test's OIDC provider: by default `executableConsole`, the Console the
// Daemon serves itself from its executable's Console build, whose sign-in
// link the executable's `console` command gives (spec #592); it has no
// single sign-on yet, so a test of one passes the server it stands in for.
// Without a server, the fixture fails with `console_not_served`.
//
// A test may put its whole folder on a size-capped filesystem of its own
// (`sizeCapped`, ADR 0122), which `goodfetch-test-env` and the Daemon see
// in their namespaces: the executable's commands run there too.
import { daemonCommand, type CliOutput } from "./daemon-command.ts";
import { daemonOf, type DaemonDescription } from "./daemon.ts";
import { TestSupportError } from "./errors.ts";
import type { OidcClient } from "./oidc-provider.ts";
import type { ProcessRecord } from "./process-record.ts";
import type { Product } from "./product-env.ts";
import type { ProvingGroundDescription } from "./proving-ground.ts";
import { sizeCapped, type SizeCapped } from "./size-capped.ts";
import { readTools, startTestEnv, toolPath, type Tools } from "./test-env.ts";

/** What a Console is served beside. */
export interface ConsoleStart {
  /** The Product whose executable runs the Daemon, and whose Console build is served. */
  readonly product: Product;
  /** The Goodfetch home of the Daemon. */
  readonly home: string;
  readonly daemon: DaemonDescription;
  /** The test's OIDC provider, for the Console's single sign-on. */
  readonly oidc: OidcClient;
  /** Runs the Daemon's executable with `args` in the test's home and cleared environment. */
  cli(args: readonly string[]): Promise<CliOutput>;
  /** The record a process the server starts joins. */
  readonly processes: ProcessRecord;
  /** The commit's executables, from the test tool manifest. */
  readonly tools: Tools;
  /** The test's cleared environment. */
  readonly env: Readonly<Record<string, string>>;
}

/** What a gateway serving the Console gives a test beside its page (ADR 0096). */
export interface GatewayAccess {
  /** The token a program signs in with at `POST /sign-in`, swapped for the gateway's cookie. */
  readonly token: string;
  /** The content origin's URL. */
  readonly content: string;
  /** The folder holding each of its engines' Goodfetch home. */
  readonly engines: string;
}

/** A Console being served. */
export interface ServedConsole {
  /** Its URL, `http://127.0.0.1:<port>/`. */
  readonly url: string;
  /** The single-user sign-in link the executable gives, each call a new one. */
  signInLink(): Promise<string>;
  /** Where the Console's single sign-on starts. */
  readonly singleSignOn: string;
  /** An address that answers 2xx to a signed-in request and 401 otherwise. */
  readonly session: string;
  /** What the gateway that serves it gives, when a gateway does; the Daemon's own Console has none. */
  readonly gateway?: GatewayAccess | undefined;
  /** Stops serving. */
  close(): Promise<void>;
}

/**
 * Serves the Console beside a started Daemon. An object rather than a
 * function, since Playwright takes a function given as an option's value
 * for the fixture that makes it.
 */
export interface ConsoleServer {
  serve(start: ConsoleStart): Promise<ServedConsole>;
}

/** The sign-in link `console --json --no-browser` prints, which opens no browser. */
async function signInLinkOf(start: ConsoleStart): Promise<string> {
  const output = await start.cli(["console", "--json", "--no-browser"]);
  let url: unknown;
  try {
    const opened: unknown = JSON.parse(output.stdout);
    url = typeof opened === "object" && opened !== null && "url" in opened ? opened.url : undefined;
  } catch {
    url = undefined;
  }
  if (output.code !== 0 || typeof url !== "string") {
    throw new TestSupportError(
      "sign_in_failed",
      `\`console --json\` gave no sign-in link (exit ${String(output.code)}): ${output.stderr}`,
    );
  }
  return url;
}

/**
 * The Console the test's Daemon serves itself, from its executable's
 * Console build (ADR 0057), at the URL its runtime file reports, its
 * sign-in link the one the executable's `console` command prints.
 */
export const executableConsole: ConsoleServer = {
  serve: async (start) => {
    const { daemon } = start;
    const url = daemon.console;
    if (url === undefined || url === null) {
      throw new TestSupportError(
        "console_not_served",
        "the test's Daemon serves no Console: its executable carries no Console build",
      );
    }
    return {
      url,
      signInLink: async () => signInLinkOf(start),
      singleSignOn: new URL("sso", url).href,
      session: new URL("session", url).href,
      close: async () => undefined,
    };
  },
};

/** A served test's Daemon, its home, and the Console it serves. */
export interface ServedDaemon extends DaemonDescription, Omit<ServedConsole, "url" | "close"> {
  readonly product: Product;
  /** The Daemon's Goodfetch home, the test's own. */
  readonly home: string;
  /** The served Console's URL, `http://127.0.0.1:<port>/`. */
  readonly url: string;
  /** The served Console's port, bound on port 0. */
  readonly port: number;
  /** The Proving ground started on the test's seed, if it asked for one. */
  readonly provingGround: ProvingGroundDescription | undefined;
  /** The executable that runs the Daemon: the test tool manifest's of `product`, or the test's `engine`. */
  readonly executable: string;
  /** The test's cleared environment, under which a command of the executable runs in the test's home. */
  readonly env: Readonly<Record<string, string>>;
  /** Runs the executable with `args` in the test's home and cleared environment, inside the size-capped filesystem's namespaces when it has one. */
  run(args: readonly string[]): Promise<CliOutput>;
  /** The size-capped filesystem the test's folder is on, when the test asked for one. */
  readonly sizeCapped: SizeCapped | undefined;
}

/** How to start a served test's Daemon. */
export interface ServedDaemonStart {
  /** The record the setup's processes join. */
  readonly processes: ProcessRecord;
  /** The test's full name, from which its seed and folder name come. */
  readonly testName: string;
  /** The test tool manifest's path: `GOODFETCH_TEST_TOOLS`, which `just test` sets. */
  readonly toolManifest: string | undefined;
  /** The Product whose executable runs the Daemon. */
  readonly product: Product;
  /** The seed of a Proving ground to start beside it, when given. */
  readonly provingGroundSeed: number | undefined;
  /** What serves the Console; `console_not_served` without one. */
  readonly consoleServer: ConsoleServer | undefined;
  readonly oidc: OidcClient;
  /** The size in bytes of a filesystem of its own to put the test's folder on, when given (Linux only). */
  readonly sizeCapped?: number | undefined;
}

/** A started served Daemon. */
export interface StartedServedDaemon {
  readonly daemon: ServedDaemon;
  /** Stops the Console, then the Daemon and the rest of the setup, which leaves no runtime file. */
  close(): Promise<void>;
}

/** Starts a served test's Daemon and its Console. */
export async function startServedDaemon(start: ServedDaemonStart): Promise<StartedServedDaemon> {
  const { consoleServer } = start;
  if (consoleServer === undefined) {
    throw new TestSupportError(
      "console_not_served",
      "no `consoleServer` serves the test's Console: keep the default, the executable's own, or pass one with `test.use`",
    );
  }
  const mounting =
    start.sizeCapped === undefined
      ? undefined
      : await sizeCapped(start.sizeCapped, toolPath(readTools(start.toolManifest), "test_env"));
  let testEnv;
  try {
    testEnv = await startTestEnv({
      processes: start.processes,
      testName: start.testName,
      toolManifest: start.toolManifest,
      program: mounting?.program,
      provingGroundSeed: start.provingGroundSeed,
      daemon: start.product,
    });
  } catch (error) {
    await mounting?.close();
    throw error;
  }
  const description = daemonOf(testEnv.description);
  const executable = toolPath(testEnv.description.tools, start.product);
  const env = testEnv.description.env;
  const pid = testEnv.pid;
  const run = async (args: readonly string[]): Promise<CliOutput> => {
    if (mounting === undefined) {
      return daemonCommand(start.processes, { executable, env }, args);
    }
    const inside = mounting.inside(pid, executable, args);
    return daemonCommand(start.processes, { executable: inside.command, env }, inside.args);
  };
  const closeTestEnv = async (): Promise<void> => {
    try {
      await testEnv.close();
    } finally {
      await mounting?.close();
    }
  };
  let served: ServedConsole;
  try {
    served = await consoleServer.serve({
      product: start.product,
      home: testEnv.description.home,
      daemon: description,
      oidc: start.oidc,
      cli: run,
      processes: start.processes,
      tools: testEnv.description.tools,
      env,
    });
  } catch (error) {
    await closeTestEnv();
    throw error;
  }
  const url = new URL(served.url);
  return {
    daemon: {
      ...description,
      product: start.product,
      home: testEnv.description.home,
      url: url.href,
      port: Number(url.port),
      provingGround: testEnv.description.proving_ground,
      executable,
      env,
      run,
      sizeCapped: mounting?.mounted(pid),
      signInLink: () => served.signInLink(),
      singleSignOn: served.singleSignOn,
      session: served.session,
      gateway: served.gateway,
    },
    close: async () => {
      try {
        await served.close();
      } finally {
        await closeTestEnv();
      }
    },
  };
}
