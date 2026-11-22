// How a Run ends and what it does meanwhile (ADRs 0029 and 0102): `finish()`
// and `stop()` answer the Run report, a failed Run raises its error and a
// call's timeout ends only its wait, while `progress()` follows the Run, on
// the Scripted engine as the Attached engine, which fails on a message it
// does not expect and on a step left unused, and writes that failure to its
// stderr, which the logger gets at warning. Python's `test_ending_a_run.py`
// and the client crate's `ending_a_run.rs` are its twins.
import {
  BrowserLaunchFailedError,
  Collie,
  DeadlineExceededError,
  type RunProgress,
} from "@goodfetch/collie";
import { failure, recordingLogger, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

describe("Ending a Run on the Scripted engine as the Attached engine", () => {
  test(
    "answers the Run report from finish and stop, raises a failed Run's error, ends only the wait on a timeout, and follows the progress",
    {
      tags: [
        "client_case/finish-stop-and-progress",
        "attached",
        "story:11-128",
        "story:11-129",
        "error:deadline_exceeded",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/finish-stop-and-progress.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const nightly = await collie.start({ runId: "nightly" });
      const progress: RunProgress[] = [];
      let set: RunProgress | undefined;
      for await (const value of nightly.progress()) {
        progress.push(value);
        if (set !== undefined) break;
        set = await nightly.setProgressMessage("three pages left");
      }
      const waited = await failure(nightly.finish({ timeout: 60_000 }));
      const finished = await nightly.finish();
      const weekly = await collie.start({ runId: "weekly" });
      const stopped = await weekly.stop();
      const broken = await collie.start({ runId: "broken" });
      const failedFinish = await failure(broken.finish());
      const failedStop = await failure(broken.stop());
      await collie.close();

      await clientCase.check(
        "the progress answers at once, then the value after it carries the program's message",
        () =>
          expect(progress.map(({ position, message }) => ({ position, message }))).toEqual([
            { position: 1, message: undefined },
            { position: 2, message: "three pages left" },
          ]),
      );
      await clientCase.check("setProgressMessage answers the progress it set", () =>
        expect(set).toMatchObject({ position: 2, message: "three pages left" }),
      );
      await clientCase.check(
        "a finish's timeout fails it with deadline_exceeded, ending only the wait",
        () => {
          expect(waited).toBeInstanceOf(DeadlineExceededError);
          expect(finished.lifecycle).toBe("finished");
        },
      );
      await clientCase.check("finish answers the whole Run report", () =>
        expect(finished).toMatchObject({
          runId: "nightly",
          lifecycle: "finished",
          verdict: "needs_review",
          verdictReasons: [{ code: "uncertain_effect", count: 1 }],
          completeness: "complete",
          usage: { requests: 3 },
          weakened: [{ key: "consent.handling", value: "off", layer: "run" }],
          plan: { status: "unchanged" },
          settings: [{ key: "timeout.close", value: 30_000, layer: "default" }],
          plugins: [{ name: "shop-login", networkLevel: "none" }],
          warnings: [{ code: "storage_low" }],
          recording: { mode: "record", file: "nightly.recording", responsesUsed: 0 },
          nothingDone: { reason: "nothing_pending" },
          uncertainEffects: [{ entry: "submit-order", since: new Date("2026-01-01T00:00:30Z") }],
          startedAt: new Date("2026-01-01T00:00:00Z"),
          endedAt: new Date("2026-01-01T00:01:00Z"),
        }),
      );
      await clientCase.check(
        "stop answers the Run stopped by its program, with its Checkpoint",
        () =>
          expect(stopped).toMatchObject({
            lifecycle: "stopped",
            stopReason: { kind: "abort", by: "program" },
            completeness: "incomplete",
            checkpoint: {
              runId: "weekly",
              journalPosition: 7,
              time: new Date("2026-01-01T00:00:05Z"),
            },
          }),
      );
      await clientCase.check("a failed Run's finish and stop raise the error that failed it", () =>
        expect(
          [failedFinish, failedStop].map((error) => error instanceof BrowserLaunchFailedError),
        ).toEqual([true, true]),
      );
      // The guest program of `nothing-at-import-or-connect` follows the
      // progress in a process of its own and finds nothing on stdout or
      // stderr but the marker warnings: no status line.
      await clientCase.check("the SDK sent exactly what the script holds", () =>
        expect(warned).toEqual([]),
      );
    },
  );
});
