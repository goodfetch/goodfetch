// Closing the tree a Collie program holds (ADR 0078): Connection, Run,
// Session and page, each closing its children first, on an Attached engine
// that plays a Scripted engine's script (ADR 0069). The script fails on a
// message it does not expect, `hello` included, and on a step left unused,
// and the engine writes that failure to its stderr, which the logger gets
// at warning: an empty `warned` says the SDK sent exactly what the script
// holds, and that the close ended the engine's input.
import { join } from "node:path";
import { Collie, RunStateUnavailableError } from "@goodfetch/collie";
import { failure, recordingLogger, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const URL = "https://shop.test/";

describe("closing a Collie Connection's values", () => {
  test(
    "closes each value's children with it, and refuses every later call on them with closed",
    { tags: ["client_case/children-close-with-their-parent", "attached", "error:closed"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/closed-with-its-children.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const run = await collie.start();
      const session = await run.newSession();
      const sessionPage = await session.newPage();

      await session.close();
      await sessionPage.close();
      const afterSession = await failure(sessionPage.goto(URL));
      const runPage = await run.newPage();
      await collie.close();
      await collie.close();
      const afterConnection = await Promise.all([
        failure(runPage.goto(URL)),
        failure(session.newPage()),
        failure(run.finish()),
      ]);

      await clientCase.check("a page closed with its Session refuses goto with closed", () =>
        expect(afterSession).toMatchObject({ code: "closed", group: "program" }),
      );
      await clientCase.check(
        "the Run's page, its Session and the Run refuse every call with closed once the Connection is",
        () =>
          expect(afterConnection).toMatchObject([
            { code: "closed", group: "program" },
            { code: "closed", group: "program" },
            { code: "closed", group: "program" },
          ]),
      );
      await clientCase.check(
        "the page's close after its Session's sent nothing, and the Connection's close aborted the Run once",
        () => expect(warned).toEqual([]),
      );
    },
  );

  test(
    "stops a running Run it closes, which a start of its Run ID resumes, and frees an ended Run's value without a call",
    { tags: ["client_case/close-stops-a-running-run", "attached"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/closed-then-resumed.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const first = await collie.start({ runId: "nightly" });

      await first.close();
      const resumed = await collie.start({ runId: "nightly" });
      const report = await resumed.finish();
      await resumed.close();
      await collie.close();

      await clientCase.check("the start after the close resumed the Run", () =>
        expect(resumed.startedAs()).toBe("resumed"),
      );
      await clientCase.check("the resumed Run finished", () =>
        expect(report).toMatchObject({ run_id: "nightly", lifecycle: "finished" }),
      );
      await clientCase.check(
        "the close of the running Run aborted it, and the close of the finished one sent nothing",
        () => expect(warned).toEqual([]),
      );
    },
  );

  test(
    "aborts its running Run, cancels a call in flight and waits for its Attached engine when it closes",
    { tags: ["client_case/connection-close-stops-its-engine", "attached", "error:call_cancelled"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/closed-during-a-finish.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });
      const run = await collie.start({ runId: "nightly" });
      const finished = failure(run.finish());

      await collie.close();

      await clientCase.check("the finish in flight fails as cancelled by its Client", async () =>
        expect(await finished).toMatchObject({
          code: "call_cancelled",
          group: "unavailable",
          details: { by: "client" },
        }),
      );
      await clientCase.check(
        "the close aborted the Run, then cancelled the finish, and its engine stopped at the end of its input",
        () => expect(warned).toEqual([]),
      );
    },
  );

  test(
    "fails its close with the engine's typed error, suppressing the error await using was unwinding from",
    {
      tags: [
        "client_case/failed-close-raises-a-typed-error",
        "attached",
        "error:run_state_unavailable",
      ],
    },
    async ({ testEnv, clientCase }) => {
      const { logger, warned } = recordingLogger();
      const [closedEnv, scopedEnv] = ["closed", "scoped"].map((name) =>
        scriptedEngineEnv(testEnv, "collie", "run/abort-refused.json", join(testEnv.folder, name)),
      );
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env: closedEnv ?? unreachable(), logger });
      await collie.start({ runId: "nightly" });
      const closed = await failure(collie.close());

      const thrown = new Error("the program failed");
      const unwound = await failure(
        (async (): Promise<void> => {
          // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
          await using scoped = await Collie.connect({ env: scopedEnv ?? unreachable(), logger });
          await scoped.start({ runId: "nightly" });
          throw thrown;
        })(),
      );

      await clientCase.check("close() fails with the refused abort's error class", () => {
        expect(closed).toBeInstanceOf(RunStateUnavailableError);
        expect(closed).toMatchObject({ code: "run_state_unavailable", group: "unavailable" });
      });
      await clientCase.check(
        "await using raises a SuppressedError: the close's error, suppressing the program's",
        () => {
          expect(unwound).toBeInstanceOf(SuppressedError);
          expect(unwound).toMatchObject({
            error: expect.any(RunStateUnavailableError),
            suppressed: thrown,
          });
        },
      );
      await clientCase.check("both scripts played to their end, so no engine wrote a failure", () =>
        expect(warned).toEqual([]),
      );
    },
  );
});

function unreachable(): never {
  throw new TypeError("invariant: two environments were made");
}
