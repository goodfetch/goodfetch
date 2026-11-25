// `Foxhound.connect()` beside a Collie Daemon of the same commit (ADR 0098):
// a Collie program on `engine: "daemon"` starts Collie's executable as the
// Daemon of the test's home and stays connected, so the Daemon is busy,
// refuses a Foxhound Client by Product and cannot hand over; `env` points
// Foxhound's SDK at Foxhound's own executable from the test tool manifest.
// The Daemon is stopped while the Collie program is still connected: once
// Foxhound's executable has run in the home, an idle Collie Daemon would
// hand over to it. Python's `test_collie_daemon.py` of Foxhound's package
// and the Rust crate's `tests/it/daemon.rs` are its twins.
import { Collie } from "@goodfetch/collie";
import { Foxhound, NotInProductError } from "@goodfetch/foxhound";
import {
  productEnv,
  startRecordingLogger,
  test,
  toolPath,
  type ProcessRecord,
  type TestEnvDescription,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** Runs `body` while a Collie program keeps the Collie Daemon it started busy, then stops that Daemon. */
async function besideABusyCollieDaemon(
  testEnv: TestEnvDescription,
  processes: ProcessRecord,
  body: (daemonUrl: string) => Promise<void>,
): Promise<void> {
  const collieEnv = productEnv(testEnv, "collie");
  // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
  const busy = await Collie.connect({ env: collieEnv, engine: "daemon" });
  try {
    await body(busy.engine().url ?? "");
  } finally {
    const stop = processes.spawn(toolPath(testEnv.tools, "collie"), ["daemon", "stop"], {
      env: collieEnv,
    });
    stop.child.stdout.resume();
    stop.child.stderr.resume();
    expect(await stop.exit).toMatchObject({ code: 0 });
    await busy.close();
  }
}

describe("Foxhound.connect() beside a busy Collie Daemon", () => {
  test(
    "with engine auto, runs Foxhound's Attached engine, says why in engine().fallback and warns once",
    { tags: ["story:11-1006", "error:not_in_product"] },
    async ({ testEnv, processes }) => {
      const env = productEnv(testEnv, "foxhound");
      const enginePath = env["GOODFETCH_FOXHOUND_ENGINE_PATH"] ?? "";
      const { logger, logged } = startRecordingLogger();

      await besideABusyCollieDaemon(testEnv, processes, async (daemonUrl) => {
        // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
        const foxhound = await Foxhound.connect({ env, logger });
        const engine = foxhound.engine();
        await foxhound.close();

        expect(engine).toMatchObject({
          kind: "attached",
          product: "foxhound",
          enginePath,
          url: null,
          fallback: {
            cause: expect.stringContaining("not_in_product"),
            fix: expect.stringContaining(`Run \`${enginePath} daemon restart\``),
          },
        });
        // The engine's own stderr lines aside, the SDK warns once.
        expect(logged.filter(([level]) => level === "warn")).toEqual([
          ["warn", expect.stringContaining(daemonUrl), expect.anything()],
        ]);
      });
    },
  );

  test(
    "with engine daemon, fails with not_in_product whose fix names daemon restart of Foxhound's executable",
    { tags: ["story:11-1006", "error:not_in_product"] },
    async ({ testEnv, processes }) => {
      const env = productEnv(testEnv, "foxhound");
      const enginePath = env["GOODFETCH_FOXHOUND_ENGINE_PATH"] ?? "";
      const { logger, logged } = startRecordingLogger();

      await besideABusyCollieDaemon(testEnv, processes, async () => {
        let error: unknown;
        try {
          // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
          await Foxhound.connect({ env, logger, engine: "daemon" });
        } catch (thrown) {
          error = thrown;
        }

        expect(error).toBeInstanceOf(NotInProductError);
        expect(error).toMatchObject({
          code: "not_in_product",
          fix: expect.stringContaining(`Run \`${enginePath} daemon restart\``),
        });
        expect(logged.filter(([level]) => level === "warn")).toEqual([]);
      });
    },
  );
});
