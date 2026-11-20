// A scope's `settings` (ADR 0083) on the Scripted engine as the Attached
// engine: `connect()`'s own reach the engine in `hello`, a Run's own sit
// over them in their three states, a key left out inheriting, `null`
// clearing and a value setting, and `explainSettings()` on the Connection
// and on the Run answers what each runs with; each setting the engine
// refuses says how to fix it, a configuration-only key passed in code
// naming the file that sets it.
import { Collie } from "@goodfetch/collie";
import {
  type ExplainedSetting,
  ProgramError,
  RefusedError,
  SettingInvalidError,
  SettingLockedError,
  SettingNotApplicableError,
  SettingUnknownError,
} from "@goodfetch/collie-client";
import { recordingLogger, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const KEYS = ["checkpoint.interval", "timeout.navigation", "timeout.wait"];

/** The Scripted engine's user configuration and Operator policy files. */
const CONFIGURATION = "/home/user/.config/goodfetch/1/configuration.toml";
const POLICY = "/home/user/.config/goodfetch/1/operator-policy.toml";

/** What `call` rejects with. */
async function failure(call: Promise<unknown>): Promise<unknown> {
  return call.then(
    () => {
      throw new TypeError("the call succeeded");
    },
    (error: unknown) => error,
  );
}

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

  test(
    "each setting the engine refuses says how to fix it, a configuration-only key naming its file",
    {
      tags: [
        "client_case/settings-refusals-say-how-to-fix",
        "attached",
        "story:11-71",
        "story:11-76",
        "error:setting_not_applicable",
        "error:setting_unknown",
        "error:setting_invalid",
        "error:setting_locked",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const { logger } = recordingLogger();
      const refusedEnv = scriptedEngineEnv(
        testEnv,
        "collie",
        "run/settings-configuration-only-key-refused.json",
      );
      const notApplicable = await failure(
        // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Collie.connect({
          env: refusedEnv,
          logger,
          // @ts-expect-error a configuration-only key is no key of `connect()`'s settings
          settings: { storage: { lowFree: 1_000 } },
        }),
      );
      const env = scriptedEngineEnv(testEnv, "collie", "run/settings-refusals-say-how-to-fix.json");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const explained = await collie.explainSettings(["storage.low_free"]);
      // @ts-expect-error a misspelled key is no key of a Run's settings
      const unknown = await failure(collie.start({ settings: { timeout: { navigations: 1_000 } } }));
      // @ts-expect-error a duration is a number of milliseconds
      const invalid = await failure(collie.start({ settings: { timeout: { wait: "soon" } } }));
      const locked = await failure(collie.start({ settings: { timeout: { navigation: 1_000 } } }));
      await collie.close();

      await clientCase.check(
        "a configuration-only key passed in code is refused at connect() with setting_not_applicable naming its file",
        () => {
          expect(notApplicable).toBeInstanceOf(SettingNotApplicableError);
          expect(notApplicable).toBeInstanceOf(ProgramError);
          expect(notApplicable).toMatchObject({
            code: "setting_not_applicable",
            group: "program",
            details: { key: "storage.low_free" },
            fix: `set \`storage.low_free\` in user configuration, \`${CONFIGURATION}\`, or an Operator policy, \`${POLICY}\``,
          });
        },
      );
      await clientCase.check(
        "explainSettings() still reports the configuration-only key, from its file",
        () =>
          expect(keyed(explained.settings)).toEqual([
            ["storage.low_free", 1_000, "user_configuration"],
          ]),
      );
      await clientCase.check("a misspelled key names its closest neighbour", () => {
        expect(unknown).toBeInstanceOf(SettingUnknownError);
        expect(unknown).toBeInstanceOf(ProgramError);
        expect(unknown).toMatchObject({
          group: "program",
          details: { key: "timeout.navigations", closest: "timeout.navigation" },
        });
      });
      await clientCase.check("a value of the wrong type names the type the setting takes", () => {
        expect(invalid).toBeInstanceOf(SettingInvalidError);
        expect(invalid).toBeInstanceOf(ProgramError);
        expect(invalid).toMatchObject({
          group: "program",
          details: {
            key: "timeout.wait",
            reason: expect.stringContaining("expected a whole number of milliseconds"),
          },
        });
      });
      await clientCase.check("a locked key names the layer that locked it", () => {
        expect(locked).toBeInstanceOf(SettingLockedError);
        expect(locked).toBeInstanceOf(RefusedError);
        expect(locked).toMatchObject({
          group: "refused",
          limit: { key: "timeout.navigation", value: 10_000, layer: "operator_policy" },
        });
      });
    },
  );
});
