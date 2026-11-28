// Integration tests for `@goodfetch/collie-langchain` (ADRs 0011, 0070, 0095,
// 0099; spec #608, issue 1380): scripts a read, an Effect, and a failing call
// through LangChain.js' agent loop (`createAgent`) with `FakeStreamingChatModel`
// and runs the same tools through LangGraph (`StateGraph` + `ToolNode`) against
// a real Attached Collie engine serving the Proving ground.
import { Collie } from "@goodfetch/collie";
import { judgeTask, taskStartUrl, test } from "@goodfetch/test-support";
import { AIMessage, HumanMessage, ToolMessage, type BaseMessage } from "@langchain/core/messages";
import type { ChatResult } from "@langchain/core/outputs";
import { FakeStreamingChatModel } from "@langchain/core/utils/testing";
import { MessagesAnnotation, StateGraph } from "@langchain/langgraph";
import { ToolNode, toolsCondition } from "@langchain/langgraph/prebuilt";
import { createAgent } from "langchain";
import { describe, expect } from "vitest";
import { collieTools } from "../src/index.ts";

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function parseLastToolPayload(
  messages: readonly BaseMessage[],
): Record<string, unknown> | undefined {
  for (let i = messages.length - 1; i >= 0; i -= 1) {
    const message = messages[i];
    if (message instanceof ToolMessage && typeof message.content === "string") {
      const parsed: unknown = JSON.parse(message.content);
      if (isRecord(parsed)) {
        return parsed;
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

class ScriptedToolChatModel extends FakeStreamingChatModel {
  private readonly decide: (messages: BaseMessage[]) => AIMessage;

  constructor(decide: (messages: BaseMessage[]) => AIMessage) {
    super({ responses: [] });
    this.decide = decide;
  }

  override bindTools(tools: Parameters<FakeStreamingChatModel["bindTools"]>[0]) {
    // Run `FakeStreamingChatModel.bindTools` to validate tool schema conversion,
    // then return a `ScriptedToolChatModel` copy so `decide` stays active
    // inside `createAgent` / `StateGraph`.
    super.bindTools(tools);
    return new ScriptedToolChatModel(this.decide);
  }

  override async _generate(messages: BaseMessage[]): Promise<ChatResult> {
    const message = this.decide(messages);
    return {
      generations: [
        {
          text: typeof message.content === "string" ? message.content : "",
          message,
        },
      ],
    };
  }
}

describe("@goodfetch/collie-langchain against a real Attached Collie engine", () => {
  test.override("provingGroundSeed", 2402);
  test.override("withBrowser", true);

  test(
    "scripts a read, an Effect, and a failing call through LangChain.js createAgent",
    { tags: ["adapter:langchain", "story:11-242"] },
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

      const model = new ScriptedToolChatModel((messages) => {
        step += 1;
        const payload = parseLastToolPayload(messages);
        if (step === 1) {
          return new AIMessage({
            content: "",
            tool_calls: [{ id: "lc1", name: "run_start", args: {}, type: "tool_call" }],
          });
        }
        if (step === 2) {
          runId = requireResultString(payload, "run_id");
          return new AIMessage({
            content: "",
            tool_calls: [
              { id: "lc2", name: "run_new_session", args: { run_id: runId }, type: "tool_call" },
            ],
          });
        }
        if (step === 3) {
          const sessionId = requireResultString(payload, "session_id");
          return new AIMessage({
            content: "",
            tool_calls: [
              {
                id: "lc3",
                name: "session_new_page",
                args: { session_id: sessionId },
                type: "tool_call",
              },
            ],
          });
        }
        if (step === 4) {
          pageId = requireResultString(payload, "page_id");
          // Effect: navigate to the Proving ground catalog page.
          return new AIMessage({
            content: "",
            tool_calls: [
              {
                id: "lc4",
                name: "page_goto",
                args: { page_id: pageId, url: productUrl },
                type: "tool_call",
              },
            ],
          });
        }
        if (step === 5) {
          // Read: extract structured fields for the Proving ground oracle.
          return new AIMessage({
            content: "",
            tool_calls: [
              {
                id: "lc5",
                name: "page_read",
                args: {
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
                },
                type: "tool_call",
              },
            ],
          });
        }
        if (step === 6) {
          const result = payload?.["result"];
          const values = isRecord(result) ? result["values"] : undefined;
          if (isRecord(values)) {
            observedRecord = values;
          }
          // Failing call: machine-only setting triggers `setting_not_applicable`.
          return new AIMessage({
            content: "",
            tool_calls: [
              {
                id: "lc6",
                name: "run_start",
                args: { settings: { "mcp.oauth.issuer": "https://example.test" } },
                type: "tool_call",
              },
            ],
          });
        }
        if (step === 7) {
          observedErrorPayload = payload;
          return new AIMessage({
            content: "",
            tool_calls: [
              { id: "lc7", name: "run_finish", args: { run_id: runId }, type: "tool_call" },
            ],
          });
        }
        return new AIMessage({ content: "langchain-done" });
      });

      const agent = createAgent({ model, tools });
      const outcome = await agent.invoke({
        messages: [new HumanMessage("Navigate, read, handle an error, and finish.")],
      });

      const answer = await judgeTask(provingGround, {
        task: "catalog/2",
        records: [observedRecord],
      });
      expect(answer.outcome).toBe("verified");
      const lastMessage = outcome.messages[outcome.messages.length - 1];
      expect(lastMessage?.content).toBe("langchain-done");
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

  test(
    "runs the same Collie tools through LangGraph StateGraph + ToolNode",
    { tags: ["adapter:langchain", "story:11-242"] },
    async ({ env, provingGround }) => {
      const startUrl = await taskStartUrl(provingGround, "statuses/1");
      // oxlint-disable-next-line no-restricted-properties -- `Collie.connect` is the SDK's own factory (ADR 0078).
      await using collie = await Collie.connect({ env });
      const tools = collieTools(collie, { profile: "browser" });

      let runId = "";
      let pageId = "";
      let step = 0;
      let observedUrl = "";

      const model = new ScriptedToolChatModel((messages) => {
        step += 1;
        const payload = parseLastToolPayload(messages);
        if (step === 1) {
          return new AIMessage({
            content: "",
            tool_calls: [{ id: "lg1", name: "run_start", args: {}, type: "tool_call" }],
          });
        }
        if (step === 2) {
          runId = requireResultString(payload, "run_id");
          return new AIMessage({
            content: "",
            tool_calls: [
              { id: "lg2", name: "run_new_session", args: { run_id: runId }, type: "tool_call" },
            ],
          });
        }
        if (step === 3) {
          const sessionId = requireResultString(payload, "session_id");
          return new AIMessage({
            content: "",
            tool_calls: [
              {
                id: "lg3",
                name: "session_new_page",
                args: { session_id: sessionId },
                type: "tool_call",
              },
            ],
          });
        }
        if (step === 4) {
          pageId = requireResultString(payload, "page_id");
          return new AIMessage({
            content: "",
            tool_calls: [
              {
                id: "lg4",
                name: "page_goto",
                args: { page_id: pageId, url: startUrl },
                type: "tool_call",
              },
            ],
          });
        }
        if (step === 5) {
          return new AIMessage({
            content: "",
            tool_calls: [
              {
                id: "lg5",
                name: "page_formats",
                args: { page_id: pageId, formats: { text: true } },
                type: "tool_call",
              },
            ],
          });
        }
        if (step === 6) {
          observedUrl = requireResultString(payload, "url");
          return new AIMessage({
            content: "",
            tool_calls: [
              { id: "lg6", name: "run_finish", args: { run_id: runId }, type: "tool_call" },
            ],
          });
        }
        return new AIMessage({ content: "langgraph-done" });
      }).bindTools(tools);

      const graph = new StateGraph(MessagesAnnotation)
        .addNode("agent", async (state) => {
          const response = await model.invoke(state.messages);
          return { messages: [response] };
        })
        .addNode("tools", new ToolNode(tools))
        .addEdge("__start__", "agent")
        .addConditionalEdges("agent", toolsCondition)
        .addEdge("tools", "agent")
        .compile();

      const outcome = await graph.invoke({
        messages: [new HumanMessage("Visit the start URL and finish.")],
      });
      const lastMessage = outcome.messages[outcome.messages.length - 1];
      expect(lastMessage?.content).toBe("langgraph-done");
      expect(observedUrl).toBe(startUrl);
    },
  );
});
