// Integration tests for `@goodfetch/collie-mastra` (ADRs 0011, 0070, 0095,
// 0099; spec #608, issue 1381): scripts a read, an Effect, and a failing call
// through a Mastra `Agent` on `MockLanguageModelV3` against a real Attached
// Collie engine serving the Proving ground.
import { Collie } from "@goodfetch/collie";
import { judgeTask, taskStartUrl, test } from "@goodfetch/test-support";
import { Agent } from "@mastra/core/agent";
import { MockLanguageModelV3 } from "ai/test";
import { describe, expect } from "vitest";
import { collieTools } from "../src/index.ts";

const EMPTY_USAGE = {
  inputTokens: { total: 10, noCache: 10, cacheRead: undefined, cacheWrite: undefined },
  outputTokens: { total: 5, text: 5, reasoning: undefined },
} as const;

function toolCallResponse(toolCallId: string, toolName: string, input: Record<string, unknown>) {
  return {
    content: [
      {
        type: "tool-call" as const,
        toolCallId,
        toolName,
        input: JSON.stringify(input),
      },
    ],
    finishReason: { unified: "tool-calls" as const, raw: undefined },
    usage: EMPTY_USAGE,
    warnings: [],
  };
}

function textResponse(text: string) {
  return {
    content: [{ type: "text" as const, text }],
    finishReason: { unified: "stop" as const, raw: undefined },
    usage: EMPTY_USAGE,
    warnings: [],
  };
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

function requireResultString(payload: Record<string, unknown> | undefined, key: string): string {
  const result = payload?.["result"];
  if (!isRecord(result) || typeof result[key] !== "string") {
    throw new TypeError(`expected string field result.${key}`);
  }
  return result[key];
}

describe("@goodfetch/collie-mastra against a real Attached Collie engine", () => {
  test.override("provingGroundSeed", 2404);
  test.override("withBrowser", true);

  test(
    "scripts a read, an Effect, and a failing call through a Mastra Agent with MockLanguageModelV3",
    { tags: ["adapter:mastra", "story:11-243"] },
    async ({ env, provingGround }) => {
      const productUrl = await taskStartUrl(provingGround, "catalog/2");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
      await using collie = await Collie.connect({ env });
      const tools = collieTools(collie, { profile: "browser" });

      let runId = "";
      let pageId = "";
      let step = 0;
      let observedRecord: Record<string, unknown> = {};
      let observedErrorPayload: Record<string, unknown> | undefined;

      const model = new MockLanguageModelV3({
        doGenerate: async ({ prompt }) => {
          step += 1;
          const payload = extractLastToolPayload(prompt);
          if (step === 1) {
            return toolCallResponse("m1", "run_start", {});
          }
          if (step === 2) {
            runId = requireResultString(payload, "run_id");
            return toolCallResponse("m2", "run_new_session", { run_id: runId });
          }
          if (step === 3) {
            const sessionId = requireResultString(payload, "session_id");
            return toolCallResponse("m3", "session_new_page", { session_id: sessionId });
          }
          if (step === 4) {
            pageId = requireResultString(payload, "page_id");
            return toolCallResponse("m4", "page_goto", { page_id: pageId, url: productUrl });
          }
          if (step === 5) {
            return toolCallResponse("m5", "page_read", {
              page_id: pageId,
              fields: {
                id: { locator: { kind: "css", selector: "span.id" } },
                name: { locator: { kind: "css", selector: "h1.name" } },
                price: {
                  locator: { kind: "css", selector: "span.price" },
                  attribute: "data-price",
                  transform: { kind: "number", locale: "en" },
                },
              },
            });
          }
          if (step === 6) {
            const result = payload?.["result"];
            const values = isRecord(result) ? result["values"] : undefined;
            if (isRecord(values)) {
              observedRecord = values;
            }
            return toolCallResponse("m6", "run_start", {
              settings: { "mcp.oauth.issuer": "https://example.test" },
            });
          }
          if (step === 7) {
            observedErrorPayload = payload;
            return toolCallResponse("m7", "run_finish", { run_id: runId });
          }
          return textResponse("mastra-done");
        },
      });

      const agent = new Agent({
        id: "collie-mastra-test",
        name: "Collie Mastra Test Agent",
        instructions: "Use Collie tools to browse, read, handle errors, and finish.",
        model,
        tools,
      });

      const result = await agent.generate("Navigate, read, handle an error, and finish.", {
        maxSteps: 12,
      });

      const answer = await judgeTask(provingGround, {
        task: "catalog/2",
        records: [observedRecord],
      });
      expect(answer.outcome).toBe("verified");
      expect(result.text).toBe("mastra-done");
      expect(observedErrorPayload).toMatchObject({
        ok: false,
        error: {
          code: "setting_not_applicable",
          group: expect.any(String),
          message: expect.stringContaining("mcp.oauth.issuer"),
          fix: expect.stringMatching(/\S/),
        },
      });
    },
  );
});
