// What `Foxhound.connect()` takes (ADRs 0083, 0088 and 0104), on an Attached
// engine of Foxhound's executable of the same commit: the program's own
// Fetch backends and Sinks, registered by name, and the Connection's
// settings, Collie's and Foxhound's, from which a key only configuration
// sets is refused, naming the files that set it, while `explainSettings()`
// still reports it. `settings.test-d.ts` holds what `tsc` refuses. Python's
// `test_connect_options.py` is its twin.
import {
  Foxhound,
  ProgramError,
  SettingNotApplicableError,
  type FetchBackendHandler,
  type SinkHandler,
} from "@goodfetch/foxhound";
import { failure, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** A Fetch backend that answers every request with an empty page. */
const emptyPage: FetchBackendHandler = (request) =>
  Promise.resolve({
    status: 200,
    headers: { "content-type": "text/html", "x-url": request.url },
    body: new Uint8Array(),
  });

/** A Sink that keeps nothing. */
const discard: SinkHandler = () => Promise.resolve();

describe("Foxhound.connect()'s options on an Attached engine", () => {
  test.override("product", "foxhound");

  test(
    "accepts the program's own Fetch backends and Sinks, each by name",
    { tags: ["attached", "story:11-1008"] },
    async ({ env }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({
        env,
        fetchBackends: { "empty-page": emptyPage },
        sinks: { discard },
        settings: { crawl: { maxDepth: 2 } },
      });
      const explained = await foxhound.explainSettings(["crawl.max_depth"]);
      await foxhound.close();

      expect(explained.settings?.map(({ key, value, layer }) => [key, value, layer])).toEqual([
        ["crawl.max_depth", 2, "run"],
      ]);
    },
  );

  test(
    "refuses a key only configuration sets with setting_not_applicable naming its files, and still explains it",
    { tags: ["attached", "story:11-1009", "error:setting_not_applicable"] },
    async ({ env }) => {
      const refused = await failure(
        // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
        Foxhound.connect({
          env,
          // @ts-expect-error a configuration-only key is no key of `connect()`'s settings
          settings: { storage: { lowFree: 1_000 } },
        }),
      );
      // oxlint-disable-next-line no-restricted-properties -- `Foxhound.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const foxhound = await Foxhound.connect({ env });
      const explained = await foxhound.explainSettings(["storage.low_free"]);
      await foxhound.close();

      expect(refused).toBeInstanceOf(SettingNotApplicableError);
      expect(refused).toBeInstanceOf(ProgramError);
      expect(refused).toMatchObject({
        code: "setting_not_applicable",
        details: { key: "storage.low_free" },
      });
      // The files sit in the home's folder of the engine's own major.
      expect(
        refused instanceof SettingNotApplicableError
          ? refused.fix?.split(env["GOODFETCH_HOME"] ?? "<no home>").join("<home>")
          : refused,
      ).toMatch(
        /^set `storage\.low_free` in user configuration, `<home>\/[^/`]+\/configuration\.toml`, or an Operator policy, `<home>\/[^/`]+\/operator-policy\.toml`$/u,
      );
      expect(explained.settings?.map(({ key, layer }) => [key, layer])).toEqual([
        ["storage.low_free", "default"],
      ]);
    },
  );
});
