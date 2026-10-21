// The Daemon of one served Console test (ADRs 0068 and 0094): a Daemon of
// its own, started by `goodfetch-test-env --daemon` in a Goodfetch home of
// its own and run by the executable of the Product whose Console build the
// test covers, with the Console it serves on a port of its own, and a
// Proving ground when the test asks for one. Tests then run in any order.
//
// The Console is served by a `ConsoleServer`, given the started Daemon and
// the test's OIDC provider. The executables serve no Console yet (area 12),
// so a test passes the server it stands in for; without one, the fixture
// fails with `console_not_served`.
import { daemonOf, type DaemonDescription } from "./daemon.ts";
import { TestSupportError } from "./errors.ts";
import type { OidcClient } from "./oidc-provider.ts";
import type { ProcessRecord } from "./process-record.ts";
import type { Product } from "./product-env.ts";
import type { ProvingGroundDescription } from "./proving-ground.ts";
import { startTestEnv, toolPath } from "./test-env.ts";

/** What a Console is served beside. */
export interface ConsoleStart {
  /** The Product whose executable runs the Daemon, and whose Console build is served. */
  readonly product: Product;
  /** The Goodfetch home of the Daemon. */
  readonly home: string;
  readonly daemon: DaemonDescription;
  /** The test's OIDC provider, for the Console's single sign-on. */
  readonly oidc: OidcClient;
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
      "the executables serve no Console yet (area 12): pass the test's `consoleServer` with `test.use`",
    );
  }
  const testEnv = await startTestEnv({
    processes: start.processes,
    testName: start.testName,
    toolManifest: start.toolManifest,
    provingGroundSeed: start.provingGroundSeed,
    daemon: start.product,
  });
  const description = daemonOf(testEnv.description);
  let served: ServedConsole;
  try {
    served = await consoleServer.serve({
      product: start.product,
      home: testEnv.description.home,
      daemon: description,
      oidc: start.oidc,
    });
  } catch (error) {
    await testEnv.close();
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
      executable: toolPath(testEnv.description.tools, start.product),
      env: testEnv.description.env,
      signInLink: () => served.signInLink(),
      singleSignOn: served.singleSignOn,
      session: served.session,
    },
    close: async () => {
      try {
        await served.close();
      } finally {
        await testEnv.close();
      }
    },
  };
}
