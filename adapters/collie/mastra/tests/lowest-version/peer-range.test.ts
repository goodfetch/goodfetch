// Lowest- and highest-version peer range check for `@goodfetch/collie-mastra`
// (ADR 0070; spec #608, issue 1382): runs `collieTools` through the pinned
// lowest `@mastra/core@1.0.0` `Agent` loop with a scripted v3 language model.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import {
  GoodfetchError,
  type Connection,
  type JSONValue,
  type RawCallOptions,
  type RawParams,
} from "@goodfetch/collie";
import { collieTools } from "@goodfetch/collie-mastra";
import { Agent } from "@mastra/core/agent";
import { describe, expect, test } from "vitest";

const localRequire = createRequire(import.meta.url);
const adapterRequire = createRequire(new URL("../../package.json", import.meta.url));

const EMPTY_USAGE = {
  inputTokens: { total: 10, noCache: 10, cacheRead: undefined, cacheWrite: undefined },
  outputTokens: { total: 5, text: 5, reasoning: undefined },
} as const;

function readPkgVersion(pkgJsonPath: string): string {
  const parsed: unknown = JSON.parse(readFileSync(pkgJsonPath, "utf8"));
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new TypeError(`expected package.json object at ${pkgJsonPath}`);
  }
  const version = Reflect.get(parsed, "version");
  if (typeof version !== "string") {
    throw new TypeError(`missing version in ${pkgJsonPath}`);
  }
  return version;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function extractLastToolPayload(
  prompt: readonly { role: string; content: unknown }[],
): Record<string, unknown> | undefined {
  for (let i = prompt.length - 1; i >= 0; i -= 1) {
    const message = prompt[i];
    if (message?.role === "tool" && Array.isArray(message.content)) {
      const part: unknown = message.content[message.content.length - 1];
      if (isRecord(part) && part["type"] === "tool-result" && isRecord(part["output"])) {
        const value = part["output"]["value"];
        if (isRecord(value)) {
          return value;
        }
        if (typeof value === "string") {
          const parsed: unknown = JSON.parse(value);
          if (isRecord(parsed)) {
            return parsed;
          }
        }
      }
    }
  }
  return undefined;
}

describe("@goodfetch/collie-mastra peer range cells", () => {
  test(
    "drives a Mastra Agent tool loop against pinned lowest peer (@mastra/core@1.0.0)",
    { tags: ["adapter:mastra", "story:11-244"] },
    async () => {
      const lowestVersion = readPkgVersion(localRequire.resolve("@mastra/core/package.json"));
      const highestVersion = readPkgVersion(adapterRequire.resolve("@mastra/core/package.json"));
      expect(lowestVersion).toBe("1.0.0");
      expect(highestVersion.startsWith("1.")).toBe(true);

      const calls: {
        operation: string;
        params: RawParams | undefined;
        options: RawCallOptions | undefined;
      }[] = [];
      const connection: Pick<Connection, "call"> = {
        call: async (operation, params, options): Promise<JSONValue> => {
          calls.push({ operation, params, options });
          if (operation === "run.finish") {
            throw new GoodfetchError("Unknown run id", {
              code: "run_not_found",
              group: "program",
              retryable: false,
              operation: "run.finish",
              fix: "Pass a live run_id from run_start.",
            });
          }
          return { run_id: "r-lowest" };
        },
      };

      const tools = collieTools(connection, { profile: "browser" });
      let step = 0;
      let observedSuccess: Record<string, unknown> | undefined;
      let observedError: Record<string, unknown> | undefined;

      const model = {
        specificationVersion: "v3" as const,
        provider: "mock-provider",
        modelId: "mock-v3",
        supportedUrls: {},
        doGenerate: async ({
          prompt,
        }: {
          prompt: readonly { role: string; content: unknown }[];
        }) => {
          step += 1;
          const payload = extractLastToolPayload(prompt);
          if (step === 1) {
            return {
              content: [
                {
                  type: "tool-call" as const,
                  toolCallId: "m1",
                  toolName: "run_start",
                  input: "{}",
                },
              ],
              finishReason: { unified: "tool-calls" as const, raw: undefined },
              usage: EMPTY_USAGE,
              warnings: [],
            };
          }
          if (step === 2) {
            observedSuccess = payload;
            return {
              content: [
                {
                  type: "tool-call" as const,
                  toolCallId: "m2",
                  toolName: "run_finish",
                  input: JSON.stringify({ run_id: "missing-run" }),
                },
              ],
              finishReason: { unified: "tool-calls" as const, raw: undefined },
              usage: EMPTY_USAGE,
              warnings: [],
            };
          }
          observedError = payload;
          return {
            content: [{ type: "text" as const, text: "lowest-mastra-loop-complete" }],
            finishReason: { unified: "stop" as const, raw: undefined },
            usage: EMPTY_USAGE,
            warnings: [],
          };
        },
        doStream: async () => {
          throw new Error("doStream not expected during Agent.generate()");
        },
      };

      const agent = new Agent({
        id: "collie-mastra-lowest",
        name: "Collie Mastra Lowest Peer Agent",
        instructions: "Execute Collie tools and report completion.",
        model,
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `collieTools` is typed against highest peer `@mastra/core@1.35.0`, while `Agent` comes from lowest peer `@mastra/core@1.0.0`.
        tools: tools as unknown as NonNullable<ConstructorParameters<typeof Agent>[0]["tools"]>,
      });

      const result = await agent.generate("Start a run and handle a failing tool call.", {
        maxSteps: 5,
      });

      expect(result.text).toBe("lowest-mastra-loop-complete");
      expect(calls.map((c) => c.operation)).toEqual(["run.start", "run.finish"]);
      expect(calls.every((c) => c.options?.form === "tool")).toBe(true);
      expect(observedSuccess).toEqual({ ok: true, result: { run_id: "r-lowest" } });
      expect(observedError).toMatchObject({
        ok: false,
        error: {
          code: "run_not_found",
          fix: "Pass a live run_id from run_start.",
        },
      });

      const lowestManifest = readFileSync(join(import.meta.dirname, "package.json"), "utf8");
      expect(lowestManifest).toContain('"@mastra/core": "1.0.0"');
    },
  );
});
