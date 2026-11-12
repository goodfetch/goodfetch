// A Run's pages on the Scripted engine (ADR 0069): each test plays a script
// of the shared corpus over the in-memory transport. The script fails the
// test on a message it does not expect and on a step left unused, so a
// member that sent a second request, or none, fails here.
import { playScript, test, type PlayedMessage } from "@goodfetch/test-support";
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

/** The error `promise` rejects with. */
async function failure(promise: Promise<unknown>): Promise<unknown> {
  try {
    await promise;
  } catch (error) {
    return error;
  }
  throw new Error("the call succeeded");
}

/** The method of each request the Client sent, in order. */
function requested(messages: readonly PlayedMessage[]): string[] {
  return messages.flatMap((played) => {
    if (played.from !== "client" || !("message" in played)) return [];
    const { message } = played;
    return typeof message === "object" &&
      message !== null &&
      "method" in message &&
      typeof message.method === "string"
      ? [message.method]
      : [];
  });
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
      expect(requested(played.messages)).toMatchInlineSnapshot(`
        [
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
        ]
      `);
    },
  );

  test(
    "refuses every call on a closed page with closed, sending nothing, and closes twice with one request",
    { tags: ["story:11-77", "error:closed"] },
    async () => {
      const { collie, played } = await collieOn("page/closed.json");
      const run = await collie.start();
      const page = await run.newPage();
      const button = page.locator("#submit");

      const closed = await page.close();
      const again = await page.close();
      const navigation = await failure(page.goto("https://20.actionability.test/"));
      const click = await failure(button.click());
      const built = await failure(page.locator("#name").fill("Goodfetch"));
      await run.finish();
      await collie.close();

      expect(again).toBe(closed);
      expect(navigation).toMatchObject({ code: "closed", group: "program" });
      expect(click).toMatchObject({ code: "closed", group: "program" });
      expect(built).toMatchObject({ code: "closed", group: "program" });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );
});
