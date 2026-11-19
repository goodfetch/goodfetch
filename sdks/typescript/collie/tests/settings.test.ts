// A scope's `settings` (ADR 0083) on the Scripted engine as the Attached
// engine: `connect()`'s own reach the engine in `hello`, a Run's own sit
// over them in their three states, a key left out inheriting, `null`
// clearing and a value setting, and `explainSettings()` on the Connection
// and on the Run answers what each runs with.
import { Collie } from "@goodfetch/collie";
import type { ExplainedSetting } from "@goodfetch/collie-client";
import { recordingLogger, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const KEYS = ["checkpoint.interval", "timeout.navigation", "timeout.wait"];

/** Each explained setting as its key, value and layer. */
function keyed(settings: readonly ExplainedSetting[]): unknown[] {
  return settings.map(({ key, value, layer }) => [key, value, layer]);
}

describe("A scope's settings on the Scripted engine as the Attached engine", () => {
  test(
    "a Run's own settings inherit, clear and set over its Connection's, as explainSettings() answers",
    { tags: ["client_case/settings-three-states", "attached", "story:11-70", "story:11-72"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/settings-three-states.json");
      const { logger } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({
        env,
        logger,
        settings: { timeout: { navigation: 1_000, wait: 3_000 }, checkpoint: { interval: 60_000 } },
      });

      const connection = await collie.explainSettings(KEYS);
      const run = await collie.start({
        settings: { timeout: { wait: 5_000 }, checkpoint: { interval: null } },
      });
      const own = await run.explainSettings(KEYS);
      await run.finish();
      await collie.close();

      await clientCase.check("the Connection explains the settings its hello carried", () =>
        expect(keyed(connection.settings)).toEqual([
          ["checkpoint.interval", 60_000, "run"],
          ["timeout.navigation", 1_000, "run"],
          ["timeout.wait", 3_000, "run"],
        ]),
      );
      await clientCase.check(
        "the Run inherits what it leaves out, clears with null and sets with a value",
        () =>
          expect(keyed(own.settings)).toEqual([
            ["checkpoint.interval", null, "run"],
            ["timeout.navigation", 1_000, "run"],
            ["timeout.wait", 5_000, "run"],
          ]),
      );
    },
  );
});
