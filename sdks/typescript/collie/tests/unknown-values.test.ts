// Unknown values from a newer engine (ADR 0079), on the Scripted engine as a
// Collie engine: each value the Client reads that no catalog of this package
// has is kept as received and raises nothing of its own. A code no catalog
// has raises its group's class alone, with its details; `call_cancelled`
// keeps a canceller and a closed value it does not know, and
// `file_write_failed` a reason no list has; and the Run's report keeps its
// Verdict, never `verified`, its Stop reason as the union's `unknown`
// member, a warning code no table has, and Foxhound's Usage kind and Limit
// scope, values of types both Products share. Played attached, and as a
// Daemon, whose script exits 0 only once every step played. Python's
// `test_unknown_values.py` is its twin.
import {
  CallCancelledError,
  Collie,
  FileWriteFailedError,
  type GoodfetchError,
  type Logger,
  PageError,
  type RunReport,
} from "@goodfetch/collie";
import {
  type ClientCaseRecord,
  goodfetchFailure,
  scriptedEngineEnv,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** The Run the script starts. */
const RUN_ID = "01ke6p4ym0j8x3v5wq2rtbn7hc";

/** A log line the case does not read. */
function ignore(): void {
  // Nothing: only warnings are kept.
}

/** A host logger at `debug` that keeps each warning, where the engine's stderr lines go. */
function keepingWarnings(): { readonly logger: Logger; readonly warned: string[] } {
  const warned: string[] = [];
  const logger: Logger = {
    level: "debug",
    debug: ignore,
    info: ignore,
    warn: (message) => {
      warned.push(message);
    },
    error: ignore,
  };
  return { logger, warned };
}

/** What the case's calls answered, in the script's order. */
interface Played {
  readonly unknownCode: GoodfetchError;
  readonly cancelled: GoodfetchError;
  readonly writeFailed: GoodfetchError;
  readonly report: RunReport;
}

/** Plays the case's calls on `collie`, then closes it. */
async function played(collie: Collie): Promise<Played> {
  const run = await collie.start();
  const page = await run.newPage();
  const unknownCode = await goodfetchFailure(page.goto("https://shop.test/"));
  const cancelled = await goodfetchFailure(page.locator("#next").click());
  const writeFailed = await goodfetchFailure(page.locator("#save").click());
  const report = await run.finish();
  await collie.close();
  return { unknownCode, cancelled, writeFailed, report };
}

/** Checks that each value no catalog of this package has was kept as the engine sent it. */
async function checkPlayed(
  clientCase: ClientCaseRecord,
  { unknownCode, cancelled, writeFailed, report }: Played,
): Promise<void> {
  await clientCase.check(
    "a code no catalog has raises its group's class alone, its code and details kept",
    () => {
      expect(unknownCode.constructor).toBe(PageError);
      expect(unknownCode).toMatchObject({
        code: "frame_frozen",
        group: "page",
        retryable: false,
        operation: "page.goto",
        callId: "call-1",
        runId: RUN_ID,
        url: "https://shop.test/",
      });
      expect(unknownCode.details).toEqual({ frozen_for: 1200 });
    },
  );
  await clientCase.check(
    "call_cancelled keeps a canceller and a closed value no catalog has, as sent",
    () => {
      expect(cancelled).toBeInstanceOf(CallCancelledError);
      expect(cancelled).toMatchObject({ code: "call_cancelled", target: "#next" });
      expect(cancelled instanceof CallCancelledError && cancelled.typedDetails).toEqual({
        by: "satellite",
        closed: "tab_group",
      });
    },
  );
  await clientCase.check("file_write_failed keeps a reason no list has, as sent", () => {
    expect(writeFailed).toBeInstanceOf(FileWriteFailedError);
    expect(writeFailed).toMatchObject({ code: "file_write_failed", target: "#save" });
    expect(writeFailed.details).toEqual({
      reason: "crosses_devices",
      os_code: "EXDEV",
      path: `/srv/goodfetch/runs/${RUN_ID}`,
      side: "engine",
    });
  });
  await clientCase.check("an unknown Verdict is kept as sent and never reads as verified", () => {
    expect(report.verdict).toBe("verified_by_quorum");
    expect(report.verdict === "verified").toBe(false);
  });
  await clientCase.check(
    "an unknown Stop reason reads as the union's unknown member, its tag and fields kept",
    () => {
      expect(report.stopReason).toEqual({ kind: "unknown", tag: "budget_window", window: "night" });
    },
  );
  await clientCase.check(
    "Foxhound's Usage kind and Limit scope, and a warning code no table has, are kept as strings",
    () => {
      expect(report.usage.other).toEqual([{ usageKind: "paid_fetch", amount: 3, currency: null }]);
      expect(report.usage.limits?.map(({ scope, usageKind }) => [scope, usageKind])).toEqual([
        [{ kind: "partitioned_crawl", id: "books" }, "paid_fetch"],
      ]);
      expect(report.warnings?.map(({ code }) => code)).toEqual(["quorum_slow"]);
    },
  );
}

describe("A Collie Connection on the Scripted engine", () => {
  test(
    "keeps each value a newer engine sends as it was sent, raising nothing of its own",
    {
      tags: ["client_case/unknown-values-are-kept", "attached", "story:11-33", "story:11-188"],
    },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/unknown-values-are-kept.json");
      const { logger, warned } = keepingWarnings();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const answered = await played(await Collie.connect({ env, logger }));

      await checkPlayed(clientCase, answered);
      await clientCase.check("the script played to its end, so its engine wrote no failure", () => {
        expect(warned).toEqual([]);
      });
    },
  );

  test(
    "keeps the same values from a Daemon",
    {
      tags: ["client_case/unknown-values-are-kept", "daemon", "story:11-33", "story:11-188"],
    },
    async ({ testEnv, processes, clientCase }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "run/unknown-values-are-kept-from-a-daemon.json",
        "socket",
      );
      const { logger } = keepingWarnings();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env: testEnv.env, engine: "daemon", logger });
      const answered = await played(collie);
      const exit = await daemon.exit;

      await checkPlayed(clientCase, answered);
      await clientCase.check(
        "the Daemon played its script through, so the Client sent exactly what it holds",
        () => expect(exit).toMatchObject({ code: 0 }),
      );
    },
  );
});
