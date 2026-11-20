// The raw catalog call (ADR 0099, G8) on the Scripted engine as a Foxhound
// engine, which a Collie program reaches: `call` on the Connection and on
// each value sends the wire's JSON, marked raw in its `_meta`, and answers
// the engine's unchanged, and the main path's protections hold.
import { Collie } from "@goodfetch/collie";
import {
  CallCancelledError,
  ClosedError,
  DeadlineExceededError,
  ProgramError,
  type RawParams,
} from "@goodfetch/collie-client";
import { recordingLogger, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

const RUN_ID = "01ke6p4ym0j8x3v5wq2rtbn7hc";

/** What `call` rejects with. */
async function failure(call: Promise<unknown>): Promise<unknown> {
  return call.then(
    () => {
      throw new TypeError("the call succeeded");
    },
    (error: unknown) => error,
  );
}

describe("The raw call on the Scripted engine as the Attached engine", () => {
  test(
    "reaches operations the SDK does not generate, on the Connection and on each value",
    { tags: ["client_case/raw-call", "attached", "story:11-181", "error:deadline_exceeded"] },
    async ({ testEnv, clientCase }) => {
      const env = scriptedEngineEnv(testEnv, "collie", "run/raw-call.json");
      const { logger, warned } = recordingLogger();
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      const collie = await Collie.connect({ env, logger });

      const listed = await collie.call("run.list", { lifecycle: "running", tags: ["nightly"] });
      const run = await collie.start();
      const failed = await run.call("run.failed_requests", { blocked: true });
      const named = await collie.call("run.failed_requests", { run_id: run.id(), blocked: false });
      const otherNamespace = await failure(run.call("page.goto", { url: "https://shop.test/" }));
      const ownParam = await failure(run.call("run.usage", { run_id: "another" }));
      const ownMeta = [
        await failure(collie.call("run.list", { _meta: { raw: false } })),
        await failure(run.call("run.usage", { _meta: {} })),
        await failure(
          collie.call(
            "run.list",
            // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- plain JavaScript, which the params' type refuses: a `toJSON` that serializes into a `_meta` of its own.
            { toJSON: () => ({ _meta: { raw: false } }) } as unknown as RawParams,
          ),
        ),
      ];
      const deadline = await failure(run.call("run.finish", {}, { timeout: 2 }));
      const stopping = new AbortController();
      const outcomes = run.call("run.outcomes", {}, { signal: stopping.signal });
      stopping.abort(new CallCancelledError("stopped by its program"));
      const cancelled = await failure(outcomes);
      const page = await run.newPage();
      const reloaded = await page.call("page.reload");
      const clicked = await page.locator("#consent").call("locator.click");
      await run.close();
      const onClosedRun = await failure(run.call("run.usage"));
      const onClosedPage = await failure(page.call("page.reload"));
      await collie.close();
      const onClosedConnection = await failure(collie.call("run.list"));

      await clientCase.check("the Connection's call answers the engine's result unchanged", () =>
        expect(listed).toEqual({
          runs: [
            {
              run_id: RUN_ID,
              location: `/runs/${RUN_ID}`,
              kind: "crawl",
              needs: "foxhound",
              tags: ["nightly"],
              lifecycle: "running",
              started_at: "2026-11-19T12:30:00Z",
              presence: "present",
              replayed: false,
            },
          ],
        }),
      );
      await clientCase.check("a value's call fills its own parameter", () =>
        expect([failed, reloaded, clicked]).toEqual([{ requests: [] }, { response: null }, {}]),
      );
      await clientCase.check("a parameter naming a second value takes its ID", () =>
        expect(named).toEqual({ requests: [] }),
      );
      await clientCase.check("an operation of another namespace fails before it is sent", () =>
        expect(otherNamespace).toMatchObject({
          constructor: ProgramError,
          code: "invalid_params",
          details: { operation: "page.goto" },
        }),
      );
      await clientCase.check("the value's own parameter is the value's to fill", () =>
        expect(ownParam).toMatchObject({ constructor: ProgramError, code: "invalid_params" }),
      );
      await clientCase.check("a `_meta` of the program's own fails before it is sent", () =>
        expect(ownMeta).toEqual([
          expect.objectContaining({ constructor: ProgramError, code: "invalid_params" }),
          expect.objectContaining({ constructor: ProgramError, code: "invalid_params" }),
          expect.objectContaining({ constructor: ProgramError, code: "invalid_params" }),
        ]),
      );
      await clientCase.check(
        "the timeout is sent as the operation's own, and its failure raises its class",
        () => expect(deadline).toBeInstanceOf(DeadlineExceededError),
      );
      await clientCase.check("the signal cancels the call", () =>
        expect(cancelled).toBeInstanceOf(CallCancelledError),
      );
      await clientCase.check("a call on a closed value fails with closed, unsent", () =>
        expect([onClosedRun, onClosedPage, onClosedConnection]).toEqual([
          expect.any(ClosedError),
          expect.any(ClosedError),
          expect.any(ClosedError),
        ]),
      );
      await clientCase.check(
        "the script played to its end, so nothing was retried or sent unasked",
        () => expect(warned).toEqual([]),
      );
    },
  );
});
