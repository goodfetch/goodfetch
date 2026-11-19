// A page's listeners on an Attached engine of Collie's executable of the
// same commit (ADRs 0069, 0081), which launches the browser of the test's
// own Browser cache: a listener gets its page's console messages in order,
// and one that throws is removed, its `listener_failed` raised once by the
// next call on its page, which is not sent and leaves the page open.
import { Collie } from "@goodfetch/collie";
import { failure, test } from "@goodfetch/test-support";
import { describe, expect, vi } from "vitest";

describe("a page's listeners on an Attached engine", () => {
  test.override("withBrowser", true);

  test(
    "hears the console in order and raises a thrown listener's listener_failed from the next call",
    {
      tags: [
        "client_case/listeners-in-order-and-a-thrower-raises-listener-failed",
        "attached",
        "story:11-61",
        "story:11-62",
        "story:11-63",
        "error:listener_failed",
      ],
    },
    async ({ env, clientCase }) => {
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078), not a browser's.
      await using collie = await Collie.connect({ env });
      const run = await collie.start();
      const page = await run.newPage();
      const texts: string[] = [];
      const bug = new Error("the listener's bug");
      let threw = false;

      const heard = page.on("console", (message) => {
        texts.push(message.text);
      });
      await page.evaluate("() => { console.log('first'); console.log('second'); }");
      await vi.waitFor(() => {
        expect(texts).toEqual(["first", "second"]);
      });
      heard.close();
      page.on("console", () => {
        threw = true;
        throw bug;
      });
      await page.evaluate("() => console.log('third')");
      await vi.waitFor(() => {
        expect(threw).toBe(true);
      });
      const refused = await failure(page.evaluate("() => 1"));
      const after = await page.evaluate("() => 1 + 1");
      await run.finish();

      await clientCase.check("the listener heard the console messages in order", () =>
        expect(texts).toEqual(["first", "second"]),
      );
      await clientCase.check(
        "the next call on the page raised listener_failed, with the listener's exception",
        () =>
          expect(refused).toMatchObject({
            code: "listener_failed",
            group: "program",
            details: { event: "page.console" },
            cause: bug,
          }),
      );
      await clientCase.check("the page stayed open and answers the call after it", () =>
        expect(after).toBe(2),
      );
    },
  );
});
