// A page's and a Session's listeners on the Scripted engine (ADR 0081): each
// test plays a script of the shared corpus over the in-memory transport,
// which fails the test on a message it does not expect and on a step left
// unused. `on` sends its call when called, with no queue in the SDK; a
// listener that throws is removed, and `listener_failed` is raised once, by
// the next call on its value or by the `close()` that closes the value or
// the Run above it. A Session's events come on the Connection's own
// Subscription, which its listeners share, rather than on their calls; each
// listener's own queue takes them, so a stalled one delays no other and,
// past `maxListenerQueue`, ends with `listener_events_lost`.
import { ClosedError, type ConsoleMessage, type Logger, type Page } from "@goodfetch/collie-client";
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

describe("a Session's listeners", () => {
  test(
    "send the Connection's Subscription, then `on`, and hear only their Session's pages, each a working Page",
    { tags: ["story:11-61", "story:11-62"] },
    async () => {
      const { collie, played } = await collieOn("page/session-listened.json");
      const run = await collie.start();
      const session = await run.newSession();
      const pages: Page[] = [];

      const subscription = session.on("page", (page) => {
        pages.push(page);
      });
      await vi.waitFor(() => {
        expect(pages).toHaveLength(1);
      });
      const [page] = pages;
      const value = await page?.evaluate("() => 1");
      subscription.close();
      await run.finish();
      await collie.close();

      expect(value).toBe(1);
      expect(played.done()).toEqual({ kind: "finished" });
      expect(protocolCounts(played.messages)).toMatchInlineSnapshot(`
        {
          "answers": 5,
          "events": 1,
          "replies": 0,
          "requests": [
            "hello",
            "run.start",
            "run.new_session",
            "events.subscribe",
            "$/credit",
            "session.on",
            "page.evaluate",
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
    "keep a stalled listener from delaying another's pages, and end it with `listener_events_lost` naming the pages it lost",
    { tags: ["story:11-63", "error:listener_events_lost"] },
    async () => {
      const played = playScript("page/session-listener-fell-behind.json");
      const errors: { readonly message: string; readonly fields: unknown }[] = [];
      const logger: Logger = {
        level: "off",
        debug: () => {},
        info: () => {},
        warn: () => {},
        error: (message, fields) => {
          errors.push({ message, fields });
        },
      };
      const collie = await connectOver(played.transport, { logger, maxListenerQueue: 2 });
      const run = await collie.start();
      const stalledSession = await run.newSession();
      const otherSession = await run.newSession();
      const { promise: stalled, resolve: release } = Promise.withResolvers<void>();
      const stalledPages: Page[] = [];
      const otherPages: Page[] = [];

      stalledSession.on("page", async (page) => {
        stalledPages.push(page);
        await stalled;
      });
      const other = otherSession.on("page", (page) => {
        otherPages.push(page);
      });
      await vi.waitFor(() => {
        expect(otherPages).toHaveLength(1);
      });
      expect(stalledPages).toHaveLength(1);
      release();
      await vi.waitFor(() => {
        expect(errors).toHaveLength(1);
      });
      other.close();
      await run.finish();
      await collie.close();

      expect(stalledPages).toHaveLength(2);
      expect(errors[0]?.fields).toMatchObject({
        event: "session.page",
        error: { code: "listener_events_lost", details: { event: "session.page", lost: 5 } },
      });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "raise a thrown listener's `listener_failed` once from the next call on its Session, which is not sent",
    { tags: ["story:11-63", "error:listener_failed"] },
    async () => {
      const { collie, played } = await collieOn("page/session-listener-failed.json");
      const run = await collie.start();
      const session = await run.newSession();
      const bug = new Error("the listener's bug");
      let threw = false;

      session.on("page", () => {
        threw = true;
        throw bug;
      });
      await vi.waitFor(() => {
        expect(threw).toBe(true);
      });
      const refused = await failure(session.pages());
      await session.close();
      await run.finish();
      await collie.close();

      expect(refused).toMatchObject({
        code: "listener_failed",
        group: "program",
        details: { event: "session.page" },
        cause: bug,
      });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );
});
