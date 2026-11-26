// A start's options (ADRs 0033, 0080, 0091, 0102 and 0105), the
// `start-takes-every-option` client conformance case. On the Scripted
// engine, which checks every message the SDK sends, a start sends its
// settings, its `timeout`, its Plan file, its Response recording and each
// Plugin change it accepts, and `timeout: null` sends `null`; a zero
// `timeout` is refused before anything is sent; and a start whose signal
// aborts once it is sent is given up with one `$/cancel` naming it. On the
// Daemon of Collie's executable of the same commit, a start's own `timeout`
// stops the Run with a Checkpoint, a start of its Run ID resumes it, and
// another Plan file or Response recording is `run_conflict` naming each.
// The Rust client crate's `start_options.rs` and Python's
// `test_start_options.py` are its twins.
import { Collie, RunConflictError, SettingInvalidError } from "@goodfetch/collie";
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

/** A zero `timeout`, refused from the argument with the fix naming its null form. */
const ZERO_TIMEOUT = {
  code: "setting_invalid",
  limit: { key: "timeout.run", value: 0, layer: "run", origin: "argument" },
  fix: expect.stringContaining("`timeout: null`"),
};

describe("A start's options on the Scripted engine as the Attached engine", () => {
  test(
    "sends every option and a null timeout, refuses a zero timeout, and gives up a start its signal aborts with $/cancel",
    {
      tags: [
        "client_case/start-takes-every-option",
        "attached",
        "story:11-50",
        "story:11-132",
        "operation:run.start",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/start-takes-every-option.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const resumed = await collie.start({
        runId: "nightly",
        settings: { timeout: { wait: 5_000 } },
        timeout: 3_600_000,
        plan: "plans/nightly.json",
        recording: { mode: "replay", path: "recordings/nightly.warc" },
        acceptPluginChange: ["shop-prices"],
      });
      await resumed.close();
      const ended = await collie.start({ runId: "done", timeout: null });
      await ended.close();
      const zero = await failure(collie.start({ runId: "never", timeout: 0 }));

      const giveUp = new AbortController();
      const givenUp = new Error("the program gave the start up");
      const slow = failure(collie.start({ runId: "slow", signal: giveUp.signal }));
      await vi.waitFor(
        () => {
          expect(
            scriptedEngineClientMethods(testEnv.home).filter((read) => read === "run.start"),
          ).toHaveLength(3);
        },
        { timeout: READ_DEADLINE_MS, interval: 10 },
      );
      giveUp.abort(givenUp);
      const cancelled = await slow;
      await collie.close();

      await clientCase.check("each start answers its Run as the engine started it", () =>
        expect([resumed.startedAs(), ended.startedAs()]).toEqual(["resumed", "ended"]),
      );
      await clientCase.check(
        "a zero timeout is setting_invalid from the argument, naming the null form",
        () => {
          expect(zero).toBeInstanceOf(SettingInvalidError);
          expect(zero).toMatchObject(ZERO_TIMEOUT);
        },
      );
      await clientCase.check(
        "a start its signal aborts rejects with the signal's reason after one $/cancel",
        () => {
          expect(cancelled).toBe(givenUp);
          expect(scriptedEngineClientMethods(testEnv.home)).toEqual([
            "hello",
            "run.start",
            "run.abort",
            "run.start",
            "run.start",
            "$/cancel",
          ]);
        },
      );
      await clientCase.check("the engine's call_cancelled answer warns of nothing", () =>
        expect(warned).toEqual([]),
      );
    },
  );
});

describe("A start's options on the Daemon of the same commit", () => {
  test.override("withDaemon", true);

  test(
    "stops a Run at its own timeout with a Checkpoint, resumes it, refuses a zero timeout and another Plan file or recording",
    {
      tags: ["client_case/start-takes-every-option", "daemon", "story:11-50", "story:11-132"],
    },
    async ({ env, clientCase }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env, engine: "daemon" });
      const timed = await collie.start({ runId: "timed", timeout: 1 });
      const timedOut = await timed.report();
      const resumed = await collie.start({ runId: "timed", timeout: 3_600_000 });
      await resumed.finish();
      const zero = await failure(collie.start({ timeout: 0 }));

      const first = await collie.start({
        runId: "planned",
        plan: "plans/a.json",
        recording: { mode: "record", path: "recordings/a.warc" },
        acceptPluginChange: ["shop-prices"],
        timeout: null,
      });
      await first.close();
      const conflict = await failure(collie.start({ runId: "planned", plan: "plans/b.json" }));

      await clientCase.check("a start's own timeout stops the Run with a Checkpoint", () =>
        expect(timedOut).toMatchObject({
          lifecycle: "stopped",
          completeness: "incomplete",
          stopReason: { kind: "timeout" },
          checkpoint: expect.anything(),
        }),
      );
      await clientCase.check("a start of its Run ID resumes it", () =>
        expect(resumed.startedAs()).toBe("resumed"),
      );
      await clientCase.check(
        "a zero timeout is setting_invalid from the argument, naming the null form",
        () => {
          expect(zero).toBeInstanceOf(SettingInvalidError);
          expect(zero).toMatchObject(ZERO_TIMEOUT);
        },
      );
      await clientCase.check(
        "another Plan file and no recording are run_conflict naming each",
        () => {
          expect(conflict).toBeInstanceOf(RunConflictError);
          expect(conflict).toMatchObject({
            code: "run_conflict",
            details: { differences: ["plan", "recording"] },
          });
        },
      );
    },
  );
});
