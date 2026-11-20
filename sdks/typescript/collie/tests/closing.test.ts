// Closing the tree a Collie program holds (ADR 0078): Connection, Run,
// Session and page, each closing its children first, on an Attached engine
// that plays a Scripted engine's script (ADR 0069). The script fails on a
// message it does not expect, `hello` included, and on a step left unused,
// and the engine writes that failure to its stderr, which the logger gets
// at warning: an empty `warned` says the SDK sent exactly what the script
// holds, and that the close ended the engine's input.
import { join } from "node:path";
import {
  ClosedError,
  Collie,
  EngineExitedError,
  type GoodfetchError,
  isGoodfetchError,
  RunStateUnavailableError,
} from "@goodfetch/collie";
import {
  failure,
  recordingLogger,
  scriptedEngineEnv,
  searchCanaries,
  startScriptedDaemon,
  test,
  type Canary,
} from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const SHOP_URL = "https://shop.test/";

/** The key `connect/exit-with-a-canary-on-stderr.json` writes in its crash line. */
const CRASH_CANARY: Canary = {
  id: "crash-model-api-key",
  kind: "model_api_key",
  value: "model-api-key-canary-7c1e4a90d2b3",
};

/** The Goodfetch error `promise` rejects with. */
async function goodfetchFailure(promise: Promise<unknown>): Promise<GoodfetchError> {
  const error = await failure(promise);
  if (!isGoodfetchError(error)) {
    throw new Error(`the call failed with ${String(error)}, no Goodfetch error`);
  }
  return error;
}

/** What a runtime's own error reads as in ADR 0079's layout: its parts, not its wording. */
function layoutOf(error: GoodfetchError): {
  readonly operation: string | undefined;
  readonly headed: string | undefined;
  readonly endsWithFix: boolean;
} {
  const head = /^([^:]+): /u.exec(error.message)?.[1];
  return {
    operation: error.operation,
    headed: head,
    endsWithFix: error.fix !== undefined && error.message.endsWith(`. ${error.fix}`),
  };
}

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
      const afterSession = await failure(sessionPage.goto(SHOP_URL));
      const runPage = await run.newPage();
      await collie.close();
      await collie.close();
      const afterConnection = await Promise.all([
        failure(runPage.goto(SHOP_URL)),
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
      await resumed.finish();
      await resumed.close();
      await collie.close();

      await clientCase.check(
        "the close of the running Run aborted it once, the start after it named its Run ID, and the close of the finished one sent nothing",
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
        "the close aborted the Run, cancelled the finish, then waited for its engine to end: the one failure, the step left for after the close, reached the logger before close() returned",
        () =>
          expect(warned).toEqual([
            "goodfetch-scripted-engine: unused_step: step 8 was left unused",
          ]),
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
      const closedEnv = scriptedEngineEnv(
        testEnv,
        "collie",
        "run/abort-refused.json",
        join(testEnv.folder, "closed"),
      );
      const scopedEnv = scriptedEngineEnv(
        testEnv,
        "collie",
        "run/abort-refused.json",
        join(testEnv.folder, "scoped"),
      );
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env: closedEnv, logger });
      await collie.start({ runId: "nightly" });
      const closed = await failure(collie.close());

      const thrown = new Error("the program failed");
      const unwound = await failure(
        (async (): Promise<void> => {
          // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
          await using scoped = await Collie.connect({ env: scopedEnv, logger });
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

  test(
    "heads each of the runtime's own errors with its call's operation in TypeScript form, its fix last",
    { tags: ["client_case/runtime-errors-in-the-callers-form", "attached"] },
    async ({ testEnv, clientCase }) => {
      const closingEnv = scriptedEngineEnv(
        testEnv,
        "collie",
        "run/closed-while-running.json",
        join(testEnv.folder, "closing"),
      );
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env: closingEnv });
      const run = await collie.start({ runId: "nightly", tags: ["shop"] });
      await collie.close();
      const runClosed = await goodfetchFailure(run.finish());
      const pageOfClosedRun = await goodfetchFailure(run.newPage());
      const connectionClosed = await goodfetchFailure(collie.start());
      const crashingEnv = scriptedEngineEnv(
        testEnv,
        "collie",
        "connect/exit-with-a-canary-on-stderr.json",
        join(testEnv.folder, "crashing"),
      );
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const crashing = await Collie.connect({ env: crashingEnv });
      const inFlight = await goodfetchFailure(crashing.start());
      const later = await goodfetchFailure(crashing.call("sample.echo", {}));
      await crashing.close();
      const errors = [runClosed, pageOfClosedRun, connectionClosed, inFlight, later];

      await clientCase.check(
        "operation holds each call's catalog name, and the message's head its TypeScript form",
        () =>
          expect(errors.map((error) => [error.operation, layoutOf(error).headed])).toEqual([
            ["run.finish", "run.finish"],
            ["run.new_page", "run.newPage"],
            ["run.start", "run.start"],
            ["run.start", "run.start"],
            ["sample.echo", "sample.echo"],
          ]),
      );
      await clientCase.check(
        "a call on a closed Run or Connection fails with closed, whose message ends with its fix",
        () =>
          expect(
            [runClosed, pageOfClosedRun, connectionClosed].map((error) => [
              error instanceof ClosedError,
              layoutOf(error).endsWithFix,
            ]),
          ).toEqual([
            [true, true],
            [true, true],
            [true, true],
          ]),
      );
      await clientCase.check(
        "the engine's exit fails the call in flight and a later call alike, each headed by its own operation",
        () => {
          expect([inFlight, later].map((error) => error instanceof EngineExitedError)).toEqual([
            true,
            true,
          ]);
          expect(inFlight.message.slice("run.start: ".length)).toBe(
            later.message.slice("sample.echo: ".length),
          );
        },
      );
      await clientCase.check(
        "the secret the engine wrote to its stderr reaches the error's details and no message",
        () => {
          expect(
            searchCanaries(
              [CRASH_CANARY],
              [{ name: "details", text: JSON.stringify(inFlight.details) }],
            ),
          ).not.toEqual([]);
          expect(
            searchCanaries(
              [CRASH_CANARY],
              errors.map((error, index) => ({ name: `message ${index}`, text: error.message })),
            ),
          ).toEqual([]);
        },
      );
    },
  );

  test(
    "heads a Daemon's dropped Connection as closed for the call in flight and each later call",
    { tags: ["client_case/runtime-errors-in-the-callers-form", "daemon"] },
    async ({ testEnv, processes, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "connect/reached-attached.json");
      const daemon = await startScriptedDaemon(
        testEnv,
        processes,
        "run/dropped-during-a-start.json",
        "socket",
      );
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env });
      const inFlight = await goodfetchFailure(collie.start());
      const later = await goodfetchFailure(collie.explainSettings());
      await collie.close();
      const exit = await daemon.exit;
      const errors = [inFlight, later];

      await clientCase.check("the Daemon played its script through", () =>
        expect(exit).toMatchObject({ code: 0 }),
      );
      await clientCase.check(
        "each call fails with closed, headed by its operation and ending with its fix",
        () =>
          expect(
            errors.map((error) => [
              error instanceof ClosedError,
              error.operation,
              layoutOf(error).headed,
              layoutOf(error).endsWithFix,
            ]),
          ).toEqual([
            [true, "run.start", "run.start", true],
            [true, "config.explain", "config.explain", true],
          ]),
      );
    },
  );
});
