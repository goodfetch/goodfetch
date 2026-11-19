// A page's listeners on the Scripted engine (ADR 0081): each test plays a
// script of the shared corpus over the in-memory transport, which fails the
// test on a message it does not expect and on a step left unused. `on` sends
// its call when called, with no queue in the SDK; a listener that throws is
// removed, and `listener_failed` is raised once, by the next call on its page
// or by the `close()` that closes the page or the Run above it.
import { ClosedError, type ConsoleMessage, type Logger } from "@goodfetch/collie-client";
import { pageOf } from "@goodfetch/collie-client/runtime";
import {
  failure,
  playScript,
  protocolCounts,
  recordingLogger,
  test,
} from "@goodfetch/test-support";
import { describe, expect, vi } from "vitest";
import type { Collie } from "./collie.ts";
import { connectOver } from "./testing.ts";

/** The Scripted engine playing `script`, and a Collie Connection opened over it. */
async function collieOn(
  script: string,
  logger?: Logger,
): Promise<{ collie: Collie; played: ReturnType<typeof playScript> }> {
  const played = playScript(script);
  const collie = await connectOver(played.transport, { logger });
  return { collie, played };
}

/** A listener that throws on its first message, and whether it did. */
function throwing(): {
  readonly listener: (message: ConsoleMessage) => void;
  readonly bug: Error;
  threw(): boolean;
} {
  const bug = new Error("the listener's bug");
  let threw = false;
  return {
    bug,
    listener: () => {
      threw = true;
      throw bug;
    },
    threw: () => threw,
  };
}

describe("a page's listeners", () => {
  test(
    "send each `on` when called, in call order with its credits, and close by cancelling it",
    { tags: ["story:11-61", "story:11-62", "story:11-63"] },
    async () => {
      const { collie, played } = await collieOn("page/listened.json");
      const run = await collie.start();
      const page = await run.newPage();
      const texts: string[] = [];

      const consoleSubscription = page.on("console", (message) => {
        texts.push(message.text);
      });
      {
        using errors = page.on("pageerror", () => {});
        await vi.waitFor(() => {
          expect(texts).toEqual(["first", "second"]);
        });
        consoleSubscription.close();
        expect(errors).toBeDefined();
      }
      await run.finish();
      await collie.close();

      expect(played.done()).toEqual({ kind: "finished" });
      expect(protocolCounts(played.messages)).toMatchInlineSnapshot(`
        {
          "answers": 4,
          "events": 1,
          "replies": 0,
          "requests": [
            "hello",
            "run.start",
            "run.new_page",
            "page.on",
            "$/credit",
            "page.on",
            "$/credit",
            "$/cancel",
            "$/cancel",
            "run.finish",
          ],
          "served": [],
        }
      `);
    },
  );

  test(
    "raise a thrown listener's `listener_failed` once from the next call on its page, which is not sent",
    { tags: ["story:11-63", "error:listener_failed"] },
    async () => {
      const { collie, played } = await collieOn("page/listener-failed.json");
      const run = await collie.start();
      const page = await run.newPage();
      const failing = throwing();

      page.on("console", failing.listener);
      await vi.waitFor(() => {
        expect(failing.threw()).toBe(true);
      });
      const refused = await failure(page.evaluate("1"));
      await page.close();
      await run.finish();
      await collie.close();

      expect(refused).toMatchObject({
        code: "listener_failed",
        group: "program",
        details: { event: "page.console" },
        cause: failing.bug,
      });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "raise a thrown listener's `listener_failed` from the page's close, once it closed the page",
    { tags: ["story:11-63", "error:listener_failed"] },
    async () => {
      const { collie, played } = await collieOn("page/listener-failed-on-close.json");
      const run = await collie.start();
      const page = await run.newPage();
      const failing = throwing();

      page.on("console", failing.listener);
      await vi.waitFor(() => {
        expect(failing.threw()).toBe(true);
      });
      const closing = await failure(page.close());
      const after = await failure(page.evaluate("1"));
      await page.close();
      await run.finish();
      await collie.close();

      expect(closing).toMatchObject({ code: "listener_failed", cause: failing.bug });
      expect(after).toMatchObject({ code: "closed" });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "raise a thrown listener's `listener_failed` from its Run's close, once it closed the Run",
    { tags: ["story:11-63", "error:listener_failed"] },
    async () => {
      const { collie, played } = await collieOn("page/listener-failed-on-run-close.json");
      const run = await collie.start();
      const page = await run.newPage();
      const failing = throwing();

      page.on("console", failing.listener);
      await vi.waitFor(() => {
        expect(failing.threw()).toBe(true);
      });
      const closing = await failure(run.close());
      await run.close();
      await collie.close();

      expect(closing).toMatchObject({ code: "listener_failed", cause: failing.bug });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "end without a log once the attempt that lent their page ends, cancelling their calls",
    { tags: ["story:11-61"] },
    async () => {
      const { logger, errored } = recordingLogger();
      const { collie, played } = await collieOn("page/listened.json", logger);
      const run = await collie.start();
      await run.newPage();
      const attempt = new AbortController();
      const page = pageOf(run, "page-1", attempt.signal);
      const texts: string[] = [];

      page.on("console", (message) => {
        texts.push(message.text);
      });
      page.on("pageerror", () => {});
      await vi.waitFor(() => {
        expect(texts).toEqual(["first", "second"]);
      });
      attempt.abort(new ClosedError("the attempt ended", {}));
      await run.finish();
      await collie.close();

      expect(played.done()).toEqual({ kind: "finished" });
      expect(errored).toEqual([]);
    },
  );
});
