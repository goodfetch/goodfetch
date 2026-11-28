// Lowest- and highest-version peer range check for `@goodfetch/collie-langchain`
// (ADR 0070; spec #608, issue 1382): runs `collieTools` through the pinned
// lowest `@langchain/core@1.1.8` `bindTools` + `FakeStreamingChatModel` tool loop.
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
import { collieTools } from "@goodfetch/collie-langchain";
import { AIMessage, HumanMessage, ToolMessage, type BaseMessage } from "@langchain/core/messages";
import type { ChatResult } from "@langchain/core/outputs";
import { FakeStreamingChatModel } from "@langchain/core/utils/testing";
import { describe, expect, test } from "vitest";

const localRequire = createRequire(import.meta.url);
const adapterRequire = createRequire(new URL("../../package.json", import.meta.url));

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

class ScriptedLowestChatModel extends FakeStreamingChatModel {
  private readonly decide: (messages: BaseMessage[]) => AIMessage;

  constructor(decide: (messages: BaseMessage[]) => AIMessage) {
    super({ responses: [] });
    this.decide = decide;
  }

  override bindTools(tools: Parameters<FakeStreamingChatModel["bindTools"]>[0]) {
    super.bindTools(tools);
    return new ScriptedLowestChatModel(this.decide);
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

describe("@goodfetch/collie-langchain peer range cells", () => {
  test(
    "drives a bound-tools agent loop against pinned lowest peer (@langchain/core@1.1.8)",
    { tags: ["adapter:langchain", "story:11-244"] },
    async () => {
      const lowestVersion = readPkgVersion(localRequire.resolve("@langchain/core/package.json"));
      const highestVersion = readPkgVersion(adapterRequire.resolve("@langchain/core/package.json"));
      expect(lowestVersion).toBe("1.1.8");
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
      const toolByName = new Map(tools.map((item) => [item.name, item]));
      let step = 0;

      const model = new ScriptedLowestChatModel(() => {
        step += 1;
        if (step === 1) {
          return new AIMessage({
            content: "",
            tool_calls: [{ id: "lc1", name: "run_start", args: {}, type: "tool_call" }],
          });
        }
        if (step === 2) {
          return new AIMessage({
            content: "",
            tool_calls: [
              {
                id: "lc2",
                name: "run_finish",
                args: { run_id: "missing-run" },
                type: "tool_call",
              },
            ],
          });
        }
        return new AIMessage({ content: "lowest-langchain-loop-complete" });
      });

      // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `collieTools` is typed against highest peer `@langchain/core@1.5.1`, while `model` comes from lowest peer `@langchain/core@1.1.8`.
      const bound = model.bindTools(tools as unknown as Parameters<typeof model.bindTools>[0]);
      const history: BaseMessage[] = [
        new HumanMessage("Start a run and handle a failing tool call."),
      ];
      const toolOutputs: unknown[] = [];

      let finalMessage: AIMessage | undefined;
      for (let turn = 0; turn < 5; turn += 1) {
        const reply = await bound.invoke(history);
        history.push(reply);
        const requested = reply.tool_calls ?? [];
        if (requested.length === 0) {
          finalMessage = reply;
          break;
        }
        for (const call of requested) {
          const target = toolByName.get(call.name);
          if (target === undefined) {
            throw new Error(`bound model requested unknown tool ${call.name}`);
          }
          const raw = await target.invoke(call.args);
          const content = typeof raw === "string" ? raw : JSON.stringify(raw);
          toolOutputs.push(JSON.parse(content));
          history.push(
            new ToolMessage({
              content,
              tool_call_id: call.id ?? call.name,
              name: call.name,
            }),
          );
        }
      }

      expect(finalMessage?.content).toBe("lowest-langchain-loop-complete");
      expect(calls.map((c) => c.operation)).toEqual(["run.start", "run.finish"]);
      expect(calls.every((c) => c.options?.form === "tool")).toBe(true);
      expect(toolOutputs[0]).toEqual({ ok: true, result: { run_id: "r-lowest" } });
      expect(toolOutputs[1]).toMatchObject({
        ok: false,
        error: {
          code: "run_not_found",
          fix: "Pass a live run_id from run_start.",
        },
      });

      const lowestManifest = readFileSync(join(import.meta.dirname, "package.json"), "utf8");
      expect(lowestManifest).toContain('"@langchain/core": "1.1.8"');
    },
  );
});
