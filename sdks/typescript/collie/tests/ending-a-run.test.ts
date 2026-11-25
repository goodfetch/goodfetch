// How a Run ends and what it does meanwhile (ADRs 0029 and 0102): `finish()`
// and `stop()` answer the Run report, a failed Run raises its error and a
// call's timeout ends only its wait, while `progress()` follows the Run: its
// first value at once, with the Human pause it waits on, then the latest
// value after it, the ones between coalesced, which carries the program's
// message. Played on the Scripted engine as the Attached engine, which fails
// on a message it does not expect and on a step left unused and writes that
// failure to its stderr, which the logger gets at warning, and as a Daemon,
// whose script exits 0 only once every step played. Python's
// `test_ending_a_run.py` and the client crate's `ending_a_run.rs` are its
// twins.
import {
  BrowserLaunchFailedError,
  Collie,
  DeadlineExceededError,
  type RunProgress,
  type RunReport,
} from "@goodfetch/collie";
import {
  type ClientCaseRecord,
  failure,
  recordingLogger,
  scriptedEngineEnv,
  startScriptedDaemon,
  test,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

/** The message the program sets. */
const MESSAGE = "three pages left";

/** What the case's program saw, in the order it called. */
interface Played {
  readonly progress: readonly RunProgress[];
  readonly set: RunProgress | undefined;
  readonly waited: unknown;
  readonly finished: RunReport;
  readonly stopped: RunReport;
  readonly failedFinish: unknown;
  readonly failedStop: unknown;
}

/** Plays the case's program on `collie`, then closes it. */
async function played(collie: Collie): Promise<Played> {
  const nightly = await collie.start({ runId: "nightly" });
  const progress: RunProgress[] = [];
  let set: RunProgress | undefined;
  for await (const value of nightly.progress()) {
    progress.push(value);
    if (set !== undefined) break;
    set = await nightly.setProgressMessage(MESSAGE);
  }
  const waited = await failure(nightly.finish({ timeout: 60_000 }));
  const finished = await nightly.finish();
  const weekly = await collie.start({ runId: "weekly" });
  const stopped = await weekly.stop();
  const broken = await collie.start({ runId: "broken" });
  const failedFinish = await failure(broken.finish());
  const failedStop = await failure(broken.stop());
  await collie.close();
  return { progress, set, waited, finished, stopped, failedFinish, failedStop };
}

/** Checks what the program saw against what the script answered. */
async function checkPlayed(
  clientCase: ClientCaseRecord,
  { progress, set, waited, finished, stopped, failedFinish, failedStop }: Played,
): Promise<void> {
  await clientCase.check(
    "the progress answers at once with the Human pause it waits on, then the latest value after it, carrying the program's message",
    () => {
      expect(progress.map(({ position, message }) => ({ position, message }))).toEqual([
        { position: 1, message: undefined },
        { position: 4, message: MESSAGE },
      ]);
      expect(progress[0]?.pauses).toEqual([
        {
          session: "shop",
          page: "login",
          reason: "Enter the code the shop sent by SMS.",
          until: "the account page loads",
          raisedBy: { by: "engine", step: "login_recovery" },
          since: new Date("2026-01-01T00:00:00Z"),
          deadline: new Date("2026-01-01T00:15:00Z"),
        },
      ]);
      expect(progress[1]?.pauses).toEqual([]);
    },
  );
  await clientCase.check("setProgressMessage answers the progress it set", () =>
    expect(set).toMatchObject({ position: 2, message: MESSAGE }),
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
      // Points in time are `Date`s and durations milliseconds (ADR 0080).
      usage: { requests: 3, browserTime: 1_500 },
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
  await clientCase.check("stop answers the Run stopped by its program, with its Checkpoint", () =>
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
}

/** The case's tags in `mode`. */
function tags(mode: "attached" | "daemon"): string[] {
  return [
    "client_case/finish-stop-and-progress",
    mode,
    "story:11-128",
    "story:11-129",
    "error:deadline_exceeded",
  ];
}

describe("Ending a Run on the Scripted engine", () => {
  test(
    "answers the Run report from finish and stop, raises a failed Run's error, ends only the wait on a timeout, and follows the progress",
    { tags: tags("attached") },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/finish-stop-and-progress.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const answered = await played(await Collie.connect({ env, logger }));

      await checkPlayed(clientCase, answered);
      // The guest program of `nothing-at-import-or-connect` follows the
      // progress in a process of its own and finds nothing on stdout or
      // stderr but the marker warnings: no status line.
      await clientCase.check("the SDK sent exactly what the script holds", () =>
        expect(warned).toEqual([]),
      );
    },
  );

  test(
    "does the same as a Daemon",
    { tags: tags("daemon") },
    async ({ testEnv, processes, clientCase }) => {
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "run/finish-stop-and-progress-from-a-daemon.json",
        "socket",
      );
      const { logger } = recordingLogger();
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
