// Cancellation (ADR 0080) on the Scripted engine: a signal's abort sends
// `$/cancel` naming the call, without waiting, and rejects it with
// `signal.reason`, never `call_cancelled`; with a signal and a timeout both
// set, whichever comes first ends the call; a click already sent is neither
// sent again nor undone (ADRs 0080 and 0081). Python's
// `test_cancellation.py` and the Rust client crate's `cancellation.rs` are
// its twins.
import { Collie } from "@goodfetch/collie";
import {
  failure,
  recordingLogger,
  scriptedEngineClientMethods,
  scriptedEngineEnv,
  test,
} from "@goodfetch/test-support";
import { describe, expect, vi } from "vitest";

/** How long the test waits for the engine to read a message. */
const READ_DEADLINE_MS = 10_000;

/**
 * Waits until the Scripted engine under `home` has read `count` calls of
 * `method`; it signals nothing, so the wait polls its record.
 */
async function untilRead(home: string, method: string, count: number): Promise<void> {
  await vi.waitFor(
    () => {
      expect(scriptedEngineClientMethods(home).filter((read) => read === method)).toHaveLength(
        count,
      );
    },
    { timeout: READ_DEADLINE_MS, interval: 10 },
  );
}

describe("A call the program cancels on the Scripted engine", () => {
  test(
    "sends $/cancel naming it, rejects with the signal's reason, and the first of signal and timeout wins",
    { tags: ["client_case/cancellation-reaches-the-engine", "attached", "story:11-47"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/cancellation-reaches-the-engine.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const run = await collie.start();
      const page = await run.newPage();

      const clickStop = new AbortController();
      const clickStopped = new Error("the program stopped the click");
      const click = failure(page.locator("#buy").click({ signal: clickStop.signal }));
      await untilRead(testEnv.home, "locator.click", 1);
      clickStop.abort(clickStopped);
      const clicked = await click;

      const stopBeforeTimeout = new AbortController();
      const finishStopped = new Error("the program stopped the finish");
      const finish = failure(run.finish({ timeout: 60_000, signal: stopBeforeTimeout.signal }));
      await untilRead(testEnv.home, "run.finish", 1);
      stopBeforeTimeout.abort(finishStopped);
      const finishedFirst = await finish;

      const stopAfterTimeout = new AbortController();
      const timedOut = await failure(run.finish({ timeout: 2, signal: stopAfterTimeout.signal }));
      stopAfterTimeout.abort(new Error("the program stopped the finish too late"));

      const report = await run.finish();
      await collie.close();

      await clientCase.check("an aborted click rejects with the signal's reason", () =>
        expect(clicked).toBe(clickStopped),
      );
      await clientCase.check(
        "a signal that comes before the timeout ends the call, rejecting with its reason",
        () => expect(finishedFirst).toBe(finishStopped),
      );
      await clientCase.check(
        "a timeout that comes first ends the call with deadline_exceeded, and the later abort sends nothing",
        () => expect(timedOut).toMatchObject({ code: "deadline_exceeded" }),
      );
      await clientCase.check(
        "each abort sent one $/cancel the engine read, and the click was neither sent again nor undone",
        () =>
          expect(scriptedEngineClientMethods(testEnv.home)).toEqual([
            "hello",
            "run.start",
            "run.new_page",
            "locator.click",
            "$/cancel",
            "run.finish",
            "$/cancel",
            "run.finish",
            "run.finish",
          ]),
      );
      await clientCase.check("the next call goes as usual", () =>
        expect(report).toMatchObject({ lifecycle: "finished" }),
      );
      await clientCase.check("the engine's call_cancelled answers warn of nothing", () =>
        expect(warned).toEqual([]),
      );
    },
  );
});
