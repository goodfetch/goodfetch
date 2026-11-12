// The Daemon `goodfetch-test-env --daemon` starts for one test (ADRs 0024,
// 0069 and 0094): the Daemon of the test's own Goodfetch home, run by the
// commit's executable of the test's Product, with the socket and token a
// Client needs to reach it. It stops, leaving no runtime file, when the
// setup is torn down. TypeScript reads what the Rust setup started, and
// starts nothing.
import { TestSupportError } from "./errors.ts";
import type { TestEnvDescription } from "./test-env.ts";

/** A started Daemon, as `goodfetch-test-env` describes it. */
export interface DaemonDescription {
  /** Where it listens: its Unix socket's path on macOS and Linux, its loopback TCP address (`127.0.0.1:<port>`) on Windows. */
  readonly socket: string;
  /** The per-launch token a Client's `hello` carries. */
  readonly token: string;
}

/** The Daemon a description names; `daemon_not_requested` when the test asked for none. */
export function daemonOf(description: TestEnvDescription): DaemonDescription {
  if (description.daemon === undefined) {
    throw new TestSupportError(
      "daemon_not_requested",
      'the test takes `daemon` but asks for none: add `test.override("withDaemon", true)`',
    );
  }
  return description.daemon;
}
