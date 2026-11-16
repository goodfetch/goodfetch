// A Run's pages on the Scripted engine (ADR 0069): each test plays a script
// of the shared corpus over the in-memory transport. The script fails the
// test on a message it does not expect and on a step left unused, so a
// member that sent a second request, or none, fails here.
import { failure, playScript, protocolCounts, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { fillTheForm } from "../tests/programs/form.ts";
import type { Collie } from "./collie.ts";
import { connectOver } from "./testing.ts";

/** The Scripted engine playing `script`, and a Collie Connection opened over it. */
async function collieOn(
  script: string,
): Promise<{ collie: Collie; played: ReturnType<typeof playScript> }> {
  const played = playScript(script);
  const collie = await connectOver(played.transport);
  return { collie, played };
}

describe("a Run's page", () => {
  test(
    "runs the form program with one request a member called",
    { tags: ["story:11-77"] },
    async () => {
      const { collie, played } = await collieOn("page/form-filled.json");

      const filled = await fillTheForm(collie, "https://20.actionability.test/");

      expect(filled).toEqual({
        status: 200,
        markdown: "done\n\nName\n\nSave",
        lifecycle: "finished",
      });
      expect(played.done()).toEqual({ kind: "finished" });
      expect(protocolCounts(played.messages)).toMatchInlineSnapshot(`
        {
          "answers": 11,
          "events": 0,
          "replies": 0,
          "requests": [
            "hello",
            "run.start",
            "run.new_page",
            "page.goto",
            "locator.fill",
            "locator.click",
            "locator.press",
            "locator.wait_for",
            "page.formats",
            "page.close",
            "run.finish",
          ],
          "served": [],
        }
      `);
    },
  );

  test(
    "refuses a call on a closed page with closed, sending nothing",
    { tags: ["story:11-77", "error:closed"] },
    async () => {
      const { collie, played } = await collieOn("page/closed.json");
      const run = await collie.start();
      const page = await run.newPage();
      await page.close();

      const navigation = await failure(page.goto("https://20.actionability.test/"));

      expect(navigation).toMatchObject({ code: "closed", group: "program" });
      await run.finish();
      await collie.close();
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test(
    "refuses an action of a Locator built on a page since closed with closed",
    { tags: ["story:11-77", "error:closed"] },
    async () => {
      const { collie, played } = await collieOn("page/closed.json");
      const run = await collie.start();
      const page = await run.newPage();
      const button = page.locator("#submit");
      await page.close();

      const click = await failure(button.click());

      expect(click).toMatchObject({ code: "closed", group: "program" });
      await run.finish();
      await collie.close();
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test("closes a page twice with one request", { tags: ["story:11-77"] }, async () => {
    const { collie, played } = await collieOn("page/closed.json");
    const run = await collie.start();
    const page = await run.newPage();
    await page.close();

    await page.close();

    await run.finish();
    await collie.close();
    expect(played.done()).toEqual({ kind: "finished" });
  });

  test(
    "closes a page of a closed Session without a request",
    { tags: ["story:11-77"] },
    async () => {
      const { collie, played } = await collieOn("page/session-closed.json");
      const run = await collie.start();
      const session = await run.newSession();
      const page = await session.newPage();
      await session.close();

      await page.close();

      await run.finish();
      await collie.close();
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );
});
