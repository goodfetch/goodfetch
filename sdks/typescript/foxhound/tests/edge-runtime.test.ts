// Edge runtimes are Clients only (ADR 0049): an edge function, here a Node
// program that defines Vercel's `EdgeRuntime` global as that runtime does,
// asks `startCrawl` for a crawl whose handlers the engine would call after
// the request, and the SDK refuses it before sending anything, so the
// Scripted engine, which accepts `hello` and expects the Client to close,
// hears no `crawl.start`. The runs on workerd and Vercel's own runtime wait
// for issue 1421's runner.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { scriptedEngineClientMethods, scriptedEngineEnv, test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";

describe("startCrawl on an edge runtime", () => {
  test(
    "fails with `edge_handlers_unsupported` before sending anything, naming a Recipe on the Daemon",
    // It declares the story alone: its record shows `hello` dispatched, so
    // the exercise audit would judge an `error:` Feature the engine never
    // sends; the unit test declares that one (TESTING.md, Feature coverage).
    { tags: ["story:11-7"] },
    async ({ testEnv, processes }) => {
      const env = scriptedEngineEnv(testEnv, "foxhound", "connect/foxhound-accepted.json");
      const out = join(testEnv.folder, "refused.json");
      const program = processes.spawn(
        process.execPath,
        ["--no-warnings", join(import.meta.dirname, "fixtures", "edge-function.ts"), out],
        { env, cwd: testEnv.folder },
      );
      const stderr: Buffer[] = [];
      program.child.stderr.on("data", (bytes: Buffer) => stderr.push(bytes));
      program.child.stdin.end();
      const exit = await program.exit;

      expect({ exit, stderr: Buffer.concat(stderr).toString() }).toMatchObject({
        exit: { code: 0, signal: null },
        stderr: "",
      });
      expect(JSON.parse(readFileSync(out, "utf8"))).toMatchObject({
        name: "EdgeHandlersUnsupportedError",
        code: "edge_handlers_unsupported",
        operation: "crawl.start",
        message: expect.stringMatching(
          /^crawl\.start: the crawl's handlers .*Recipe on the Daemon/,
        ),
        fix: expect.stringContaining("Recipe on the Daemon"),
        details: { runtime: "vercel_edge", operation: "crawl.start" },
      });
      expect(scriptedEngineClientMethods(testEnv.home)).toEqual(["hello"]);
    },
  );
});
