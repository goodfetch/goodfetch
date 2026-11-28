// Integration tests for `@goodfetch/collie-ai-sdk` (ADRs 0011, 0070, 0095,
// 0099; spec #608, issue 1378): scripts a read, an Effect, and a failing call
// through Vercel AI SDK's `generateText` and `ToolLoopAgent` with
// `MockLanguageModelV4` against a real Attached Collie engine serving the
// Proving ground.
import { Collie } from "@goodfetch/collie";
import { judgeTask, taskStartUrl, test } from "@goodfetch/test-support";
import { generateText, stepCountIs, ToolLoopAgent } from "ai";
import { MockLanguageModelV4 } from "ai/test";
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

function requireString(record: unknown, key: string): string {
  if (!isRecord(record) || typeof record[key] !== "string") {
    throw new TypeError(`expected string field ${key}`);
  }
  return record[key];
}

function extractLastToolResult(
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

describe("@goodfetch/collie-ai-sdk against a real Attached Collie engine", () => {
  test.override("provingGroundSeed", 2401);
  test.override("withBrowser", true);

  test(
    "scripts a read, an Effect, and a failing call through generateText with MockLanguageModelV4",
    { tags: ["adapter:ai-sdk", "story:11-241", "story:11-245", "story:11-246"] },
    async ({ env, provingGround }) => {
      const productUrl = await taskStartUrl(provingGround, "catalog/2");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
      await using collie = await Collie.connect({ env });
      const tools = collieTools(collie, { profile: "browser" });

      let runId = "";
      let pageId = "";
      let step = 0;
      let observedErrorOutput: { type: string; value: unknown } | undefined;
      let observedRecord: Record<string, unknown> = {};

      const model = new MockLanguageModelV4({
        doGenerate: async ({ prompt }) => {
          step += 1;
          const lastResult = extractLastToolResult(prompt);
          if (step === 1) {
            return toolCallResponse("c1", "run_start", {});
          }
          if (step === 2) {
            runId = requireString(lastResult?.value, "run_id");
            return toolCallResponse("c2", "run_new_session", { run_id: runId });
          }
          if (step === 3) {
            const sessionId = requireString(lastResult?.value, "session_id");
            return toolCallResponse("c3", "session_new_page", { session_id: sessionId });
          }
          if (step === 4) {
            pageId = requireString(lastResult?.value, "page_id");
            // Effect: navigate to the Proving ground catalog page.
            return toolCallResponse("c4", "page_goto", { page_id: pageId, url: productUrl });
          }
          if (step === 5) {
            // Read: extract structured fields for the Proving ground oracle.
            return toolCallResponse("c5", "page_read", {
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
            const values = isRecord(lastResult?.value) ? lastResult.value["values"] : undefined;
            if (isRecord(values)) {
              observedRecord = values;
            }
            // Failing call: machine-only setting triggers `setting_not_applicable` with `fix`
            // rendered in `tool` form (`mcp.oauth.issuer`, not camelCase).
            return toolCallResponse("c6", "run_start", {
              settings: { "mcp.oauth.issuer": "https://example.test" },
            });
          }
          if (step === 7) {
            observedErrorOutput = lastResult;
            return toolCallResponse("c7", "run_finish", { run_id: runId });
          }
          return textResponse("done");
        },
      });

      const result = await generateText({
        model,
        tools,
        prompt: "Navigate, read, handle an error, and finish the run.",
        stopWhen: stepCountIs(12),
      });

      const answer = await judgeTask(provingGround, {
        task: "catalog/2",
        records: [observedRecord],
      });
      expect(answer.outcome).toBe("verified");
      expect(result.text).toBe("done");
      expect(observedErrorOutput).toMatchObject({
        type: "error-json",
        value: {
          code: "setting_not_applicable",
          group: expect.any(String),
          message: expect.stringContaining("mcp.oauth.issuer"),
          fix: expect.stringMatching(/\S/),
        },
      });
    },
  );

  test(
    "drives a read through ToolLoopAgent with MockLanguageModelV4",
    { tags: ["adapter:ai-sdk", "story:11-241", "story:11-246"] },
    async ({ env, provingGround }) => {
      const startUrl = await taskStartUrl(provingGround, "statuses/1");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
      await using collie = await Collie.connect({ env });
      const tools = collieTools(collie, { profile: "browser" });

      let runId = "";
      let pageId = "";
      let step = 0;
      let observedUrl = "";

      const model = new MockLanguageModelV4({
        doGenerate: async ({ prompt }) => {
          step += 1;
          const lastResult = extractLastToolResult(prompt);
          if (step === 1) {
            return toolCallResponse("a1", "run_start", {});
          }
          if (step === 2) {
            runId = requireString(lastResult?.value, "run_id");
            return toolCallResponse("a2", "run_new_session", { run_id: runId });
          }
          if (step === 3) {
            const sessionId = requireString(lastResult?.value, "session_id");
            return toolCallResponse("a3", "session_new_page", { session_id: sessionId });
          }
          if (step === 4) {
            pageId = requireString(lastResult?.value, "page_id");
            return toolCallResponse("a4", "page_goto", { page_id: pageId, url: startUrl });
          }
          if (step === 5) {
            return toolCallResponse("a5", "page_formats", {
              page_id: pageId,
              formats: { text: true },
            });
          }
          if (step === 6) {
            observedUrl = requireString(lastResult?.value, "url");
            return toolCallResponse("a6", "run_finish", { run_id: runId });
          }
          return textResponse("agent-done");
        },
      });

      const agent = new ToolLoopAgent({
        model,
        tools,
        stopWhen: stepCountIs(10),
      });

      const result = await agent.generate({ prompt: "Visit the start URL and finish." });
      expect(result.text).toBe("agent-done");
      expect(observedUrl).toBe(startUrl);
    },
  );
});
