// `Foxhound.connect()` itself, as far as its engine's process (ADR 0069):
// the Scripted engine started as an Attached engine, which `env` points at
// with `GOODFETCH_FOXHOUND_ENGINE_PATH` and which plays the script of the
// test's own Goodfetch home. The tracer program is the one the unit tests
// run over the in-memory transport.
import { Foxhound, NotInProductError } from "@goodfetch/foxhound";
import {
  recordingLogger,
  scriptedEngineEnv,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { processHandlers } from "./process-handlers.ts";
import { startAndFinish } from "./programs/tracer.ts";

describe("Foxhound.connect() on an Attached engine", () => {
  test(
    "has finished hello with the engine Foxhound's variable names, whatever Collie's names, and installs no handler",
    { tags: ["story:11-1002", "story:11-1007"] },
    async ({ testEnv }) => {
      const env: Readonly<Record<string, string>> = {
        ...scriptedEngineEnv(testEnv, "foxhound", "run/foxhound-started-and-finished.json"),
        GOODFETCH_COLLIE_ENGINE_PATH: "/nonexistent/goodfetch-collie",
      };
      const { logger, warned } = recordingLogger();
      const before = processHandlers();

      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env, logger });
      const whileOpen = processHandlers();
      const engine = foxhound.engine();
      const traced = await startAndFinish(foxhound);

      expect(engine).toEqual({
        kind: "attached",
        product: "foxhound",
        version: "0.0.0",
        protocolVersion: "0.1",
        projectFile: null,
        home: env["GOODFETCH_HOME"],
        enginePath: env["GOODFETCH_FOXHOUND_ENGINE_PATH"],
        url: null,
        fallback: null,
      });
      expect(traced.report).toMatchObject({ lifecycle: "finished", verdict: "verified" });
      expect([whileOpen, processHandlers()]).toEqual([before, before]);
      // The script played to its end, so its engine wrote no failure.
      expect(warned).toEqual([]);
    },
  );

  test(
    "sends Limits on Paid fetches, Foxhound's own Usage kind, at connect() and at a Run's start",
    { tags: ["story:11-51"] },
    async ({ testEnv }) => {
      const env = scriptedEngineEnv(testEnv, "foxhound", "run/foxhound-paid-fetch-limits.json");
      const { logger, warned } = recordingLogger();

      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env, logger, limits: { paidFetches: 3 } });
      const run = await foxhound.start({
        limits: { price: { paidFetches: { amount: "0.75", currency: "USD" } } },
      });
      const report = await run.finish();
      await foxhound.close();

      expect(report).toMatchObject({ lifecycle: "finished" });
      // The engine read each Limit as the script expects, so it wrote no failure.
      expect(warned).toEqual([]);
    },
  );

  test(
    "fails with not_in_product when a Collie engine answers, which writes no failure of its own",
    { tags: ["story:11-1005", "error:not_in_product"] },
    async ({ testEnv }) => {
      const env = scriptedEngineEnv(testEnv, "foxhound", "connect/foxhound-refused-by-collie.json");
      const { logger, warned } = recordingLogger();

      let error: unknown;
      try {
        // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
        await Foxhound.connect({ env, logger });
      } catch (thrown) {
        error = thrown;
      }

      expect(error).toBeInstanceOf(NotInProductError);
      expect(error).toMatchObject({
        code: "not_in_product",
        details: { needs: "foxhound", product: "collie", product_version: "0.0.0" },
        fix: expect.stringContaining("npm install @goodfetch/foxhound"),
      });
      expect(warned).toEqual([]);
    },
  );

  test(
    "with engine auto, runs an Attached Foxhound when a Collie Daemon refuses it with not_in_product, and warns once",
    {
      tags: [
        "client_case/connect-falls-back-when-the-daemon-refuses",
        "attached",
        "story:11-9",
        "story:11-290",
        "error:not_in_product",
      ],
    },
    async ({ testEnv, processes, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "foxhound", "connect/foxhound-accepted.json");
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "connect/daemon-refused-not-in-product.json",
        "socket",
      );
      const { logger, warned } = recordingLogger();

      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env, logger });
      const engine = foxhound.engine();
      await foxhound.close();

      await clientCase.check(
        "the Collie Daemon played its script through, refusing hello",
        async () => expect(await daemon.exit).toMatchObject({ code: 0 }),
      );
      // No newer Foxhound is installed, so Foxhound's executable is its
      // major's pick and the fix adds its `daemon restart` alone.
      await clientCase.check(
        "engine() reports the Attached Foxhound, and the fallback's cause and fix",
        () =>
          expect(engine).toMatchObject({
            kind: "attached",
            product: "foxhound",
            enginePath: env["GOODFETCH_FOXHOUND_ENGINE_PATH"],
            url: null,
            fallback: {
              cause: expect.stringContaining("not_in_product"),
              fix: `Connect to a Foxhound engine, or run with Foxhound, which contains everything Collie does (\`npm install @goodfetch/foxhound\`). Run \`${env["GOODFETCH_FOXHOUND_ENGINE_PATH"] ?? ""} daemon restart\` to restart the Daemon on its major's pick.`,
            },
          }),
      );
      await clientCase.check("the fallback is logged once at warning", () =>
        expect(warned).toEqual([expect.stringContaining(daemon.address)]),
      );
    },
  );
});
