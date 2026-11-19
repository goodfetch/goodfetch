// ADR 0030's run-time warning from what no generated member sends (ADR
// 0099): the hand-written `start` warns of `run.start`'s markers, and `on`
// of the event it listens to, once per process. Its own file, so no other
// test of the process warned first.
import { playScript, test } from "@goodfetch/test-support";
import { expect, vi } from "vitest";
import { connectOver } from "./testing.ts";

/** The warning an Experimental entry `name` gives, as the warning listener records it. */
function experimental(name: string): string {
  return `ExperimentalWarning: \`${name}\` is Experimental: outside semver until it is promoted (ADR 0030).`;
}

test(
  "a hand-written member and a listener warn of their entry's markers, once per process",
  { tags: ["story:11-184"] },
  async () => {
    const warnings: string[] = [];
    const listen = (warning: Error): void => {
      if ("code" in warning && String(warning.code).startsWith("GOODFETCH_")) {
        warnings.push(`${warning.name}: ${warning.message}`);
      }
    };
    process.on("warning", listen);
    try {
      const played = playScript("page/listened.json");
      const collie = await connectOver(played.transport);
      const run = await collie.start();
      const page = await run.newPage();
      const texts: string[] = [];
      const consoleSubscription = page.on("console", (message) => {
        texts.push(message.text);
      });
      const errorsSubscription = page.on("pageerror", () => {});
      await vi.waitFor(() => {
        expect(texts).toEqual(["first", "second"]);
      });
      consoleSubscription.close();
      errorsSubscription.close();
      await run.finish();
      await collie.close();
      expect(played.done()).toEqual({ kind: "finished" });
    } finally {
      process.off("warning", listen);
    }

    expect(warnings).toEqual(
      expect.arrayContaining([
        experimental("run.start"),
        experimental("page.console"),
        experimental("page.page_error"),
      ]),
    );
    expect(warnings.filter((warning) => warning === experimental("run.start"))).toHaveLength(1);
  },
);
