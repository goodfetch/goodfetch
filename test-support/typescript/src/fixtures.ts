// The Vitest fixtures of `@goodfetch/test-support` (ADR 0062), through
// `test.extend`: a test takes `testEnv` or `env` and gets the same per-test
// setup as a Rust test, from the real `goodfetch-test-env`.
//
// - `testEnv`: the description `goodfetch-test-env` prints, validated
//   against the committed schema; its teardown closes that process's stdin.
// - `env`: the environment map an SDK takes: the description's cleared
//   environment with the engine-path variable of the test's Product
//   (ADRs 0069, 0090 and 0094).
// - `processes`: the test's process record; every process started through
//   it, `goodfetch-test-env` included, must have ended by teardown.
// - `provingGround`: the Proving ground `goodfetch-test-env` started on
//   `provingGroundSeed`, with its `resolve` mapping, certificate authority
//   and network fixtures; it stops when the setup is torn down.
// - `daemon`: the Daemon `goodfetch-test-env` started in the test's home
//   when `withDaemon` is set, run by the executable of the test's
//   `product`, with its socket and token; it stops, leaving no runtime
//   file, when the setup is torn down.
// - `clientCase`: the record of the Client conformance case the test's
//   `client_case/<id>` tag names, whose checks and evidence the case-log
//   reporter writes (ADR 0069).
//
// Options a test overrides with `test.override`: `product` (Collie's unless
// the test needs Foxhound), `provingGroundSeed`, `withDaemon`,
// `withBrowser` (the home's Browser cache laid out, and the Proving ground's
// hosts reachable, for a program whose engine opens a page),
// `browserDownload` (an empty Browser cache, whose missing build the
// engine downloads from the Proving ground or cannot; a test that asks for
// `read-only-cache` is skipped on Windows), `toolManifest` and
// `testEnvProgram`.
import { test as base, type TestAPI } from "vitest";
import { clientCaseRecord, type ClientCaseRecord } from "./case-log.ts";
import { daemonOf, type DaemonDescription } from "./daemon.ts";
import { ProcessRecord } from "./process-record.ts";
import { productEnv, type Product } from "./product-env.ts";
import { provingGroundOf, type ProvingGroundDescription } from "./proving-ground.ts";
import {
  startTestEnv,
  type BrowserDownload,
  type Program,
  type TestEnvDescription,
} from "./test-env.ts";

/** Why a test that asks for a read-only Browser cache is skipped on Windows. */
const READ_ONLY_CACHE_ON_WINDOWS =
  "`goodfetch-test-env` makes a cache read-only with its folder's mode, which a Windows folder lacks";

/** What a test takes from test support's `test`. */
export interface TestSupportFixtures {
  /** The Product whose executable `env` points the SDK at: Collie's unless the test needs Foxhound. */
  product: Product;
  /** The test tool manifest's path: `GOODFETCH_TEST_TOOLS`, which `just test` sets. */
  toolManifest: string | undefined;
  /** What runs in place of the manifest's `goodfetch-test-env`, when set. */
  testEnvProgram: Program | undefined;
  /** The seed of the test's Proving ground, which `provingGround` needs. */
  provingGroundSeed: number | undefined;
  /** Whether the test's setup starts the Daemon of `product`'s executable in its home, which `daemon` needs. */
  withDaemon: boolean;
  /** Whether the test's home is prepared for an engine that launches a browser and reaches the Proving ground. */
  withBrowser: boolean;
  /** How the engine's first use of an empty Browser cache goes, with the Proving ground; the cache is laid out without it. */
  browserDownload: BrowserDownload | undefined;
  /** The test's process record. */
  processes: ProcessRecord;
  /** The test's setup, as `goodfetch-test-env` describes it. */
  testEnv: TestEnvDescription;
  /** The environment map an SDK takes: `GOODFETCH_HOME` and the Product's engine-path variable among it. */
  env: Readonly<Record<string, string>>;
  /** The Proving ground started on `provingGroundSeed`; `proving_ground_unseeded` without one. */
  provingGround: ProvingGroundDescription;
  /** The Daemon started in the test's home with `withDaemon`; `daemon_not_requested` without it. */
  daemon: DaemonDescription;
  /** The record of the test's Client conformance case; `client_case_untagged` without a `client_case/` tag. */
  clientCase: ClientCaseRecord;
}

/** Vitest's `test` with test support's fixtures. */
export const test: TestAPI<TestSupportFixtures> = base.extend<TestSupportFixtures>({
  product: "collie",
  toolManifest: process.env["GOODFETCH_TEST_TOOLS"],
  testEnvProgram: undefined,
  provingGroundSeed: undefined,
  withDaemon: false,
  withBrowser: false,
  browserDownload: undefined,
  // oxlint-disable-next-line no-empty-pattern -- Vitest reads a fixture's dependencies from its first parameter, which must be a destructuring pattern.
  processes: async ({}, use) => {
    const processes = new ProcessRecord();
    await use(processes);
    await processes.close();
  },
  testEnv: async (
    {
      task,
      skip,
      processes,
      toolManifest,
      testEnvProgram,
      provingGroundSeed,
      withDaemon,
      withBrowser,
      browserDownload,
      product,
    },
    use,
  ) => {
    // At run time, as a skipIf would leave the test out of the case list's
    // listing on every platform.
    if (browserDownload === "read-only-cache" && process.platform === "win32") {
      skip(READ_ONLY_CACHE_ON_WINDOWS);
    }
    const started = await startTestEnv({
      processes,
      testName: task.fullName,
      toolManifest,
      program: testEnvProgram,
      provingGroundSeed,
      daemon: withDaemon ? product : undefined,
      browser: withBrowser,
      browserDownload,
    });
    await use(started.description);
    await started.close({ failed: task.result?.state === "fail" });
  },
  env: async ({ testEnv, product }, use) => {
    await use(productEnv(testEnv, product));
  },
  provingGround: async ({ testEnv }, use) => {
    await use(provingGroundOf(testEnv));
  },
  daemon: async ({ testEnv }, use) => {
    await use(daemonOf(testEnv));
  },
  clientCase: async ({ task }, use) => {
    await use(clientCaseRecord(task));
  },
});
