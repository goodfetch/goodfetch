// Lowest- and highest-version peer range check for `@goodfetch/collie-ai-sdk`
// (ADR 0070; spec #608, issues 1378 and 1382): runs `collieTools` through the
// pinned lowest `ai@7.0.0` agent loop (`generateText` + `MockLanguageModelV4`).
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
import { collieTools } from "@goodfetch/collie-ai-sdk";
import { generateText, stepCountIs } from "ai";
import { MockLanguageModelV4 } from "ai/test";
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

function extractLastToolOutput(
  prompt: readonly { role: string; content: unknown }[],
): { type: string; value: unknown } | undefined {
  for (let i = prompt.length - 1; i >= 0; i -= 1) {
    const message = prompt[i];
    if (message?.role === "tool" && Array.isArray(message.content)) {
      const part: unknown = message.content[message.content.length - 1];
      if (
        isRecord(part) &&
        part["type"] === "tool-result" &&
        isRecord(part["output"]) &&
        typeof part["output"]["type"] === "string"
      ) {
        return { type: part["output"]["type"], value: part["output"]["value"] };
      }
    }
  }
  return undefined;
}

describe("@goodfetch/collie-ai-sdk peer range cells", () => {
  test(
    "drives a multi-step generateText tool loop against pinned lowest peer (ai@7.0.0)",
    { tags: ["adapter:ai-sdk", "story:11-244"] },
    async () => {
      const lowestVersion = readPkgVersion(localRequire.resolve("ai/package.json"));
      const highestVersion = readPkgVersion(adapterRequire.resolve("ai/package.json"));
      expect(lowestVersion).toBe("7.0.0");
      expect(highestVersion.startsWith("7.")).toBe(true);

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
      let observedSuccess: { type: string; value: unknown } | undefined;
      let observedError: { type: string; value: unknown } | undefined;

      const model = new MockLanguageModelV4({
        doGenerate: async ({ prompt }) => {
          step += 1;
          const last = extractLastToolOutput(prompt);
          if (step === 1) {
            return {
              content: [
                { type: "tool-call", toolCallId: "l1", toolName: "run_start", input: "{}" },
              ],
              finishReason: { unified: "tool-calls", raw: undefined },
              usage: EMPTY_USAGE,
              warnings: [],
            };
          }
          if (step === 2) {
            observedSuccess = last;
            return {
              content: [
                {
                  type: "tool-call",
                  toolCallId: "l2",
                  toolName: "run_finish",
                  input: JSON.stringify({ run_id: "missing-run" }),
                },
              ],
              finishReason: { unified: "tool-calls", raw: undefined },
              usage: EMPTY_USAGE,
              warnings: [],
            };
          }
          observedError = last;
          return {
            content: [{ type: "text", text: "lowest-loop-complete" }],
            finishReason: { unified: "stop", raw: undefined },
            usage: EMPTY_USAGE,
            warnings: [],
          };
        },
      });

      const result = await generateText({
        model,
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `collieTools` is typed against highest peer `ai@7.0.99`, while `generateText` comes from lowest peer `ai@7.0.0`.
        tools: tools as unknown as NonNullable<Parameters<typeof generateText>[0]["tools"]>,
        prompt: "Start a run and handle a failing tool call.",
        stopWhen: stepCountIs(5),
      });

      expect(result.text).toBe("lowest-loop-complete");
      expect(calls.map((c) => c.operation)).toEqual(["run.start", "run.finish"]);
      expect(calls.every((c) => c.options?.form === "tool")).toBe(true);
      expect(observedSuccess).toEqual({ type: "json", value: { run_id: "r-lowest" } });
      expect(observedError?.type).toBe("error-json");
      expect(observedError?.value).toMatchObject({
        code: "run_not_found",
        fix: "Pass a live run_id from run_start.",
      });

      const lowestManifest = readFileSync(join(import.meta.dirname, "package.json"), "utf8");
      expect(lowestManifest).toContain('"ai": "7.0.0"');
    },
  );
});
