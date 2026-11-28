// Unit tests for `@goodfetch/collie-ai-sdk` (ADRs 0011, 0070, 0071, 0095, 0099;
// spec #608, issue 1378): tool snapshots, generated-surface kept-off check,
// form `tool` + no-retry check, and clean-process import check.
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import {
  ProgramError,
  type Connection,
  type JSONValue,
  type RawCallOptions,
  type RawParams,
} from "@goodfetch/collie";
import {
  COLLIE_MCP_PROFILES,
  KEPT_OFF_OPERATIONS,
  type McpProfile,
} from "@goodfetch/collie-client/mcp-tools";
import { asSchema, generateText, stepCountIs } from "ai";
import { MockLanguageModelV4 } from "ai/test";
import { describe, expect, test } from "vitest";
import { collieTools } from "./index.ts";

const ALL_PROFILES: readonly McpProfile[] = ["browser", "agent", "run", "code", "full"];

function stubConnection(
  onCall: (operation: string, params?: RawParams, options?: RawCallOptions) => Promise<JSONValue>,
): Pick<Connection, "call"> {
  return { call: onCall };
}

describe("@goodfetch/collie-ai-sdk", () => {
  test(
    "snapshots the converted AI SDK tools per profile and maps readOnly metadata",
    { tags: ["adapter:ai-sdk", "story:11-245"] },
    async () => {
      const connection = stubConnection(async () => ({}));
      const summary: Record<
        string,
        readonly {
          name: string;
          description: string | undefined;
          readOnly: unknown;
          operationClass: unknown;
          hasObjectSchema: boolean;
        }[]
      > = {};

      for (const profile of ALL_PROFILES) {
        const tools = collieTools(connection, { profile });
        const entries = [];
        for (const [name, definition] of Object.entries(tools)) {
          const schema = await asSchema(definition.inputSchema).jsonSchema;
          entries.push({
            name,
            description:
              typeof definition.description === "string" ? definition.description : undefined,
            readOnly: definition.metadata?.["readOnly"],
            operationClass: definition.metadata?.["operationClass"],
            hasObjectSchema: schema.type === "object",
          });
        }
        summary[profile] = entries;
      }

      // Default profile matches `browser`.
      expect(Object.keys(collieTools(connection))).toEqual(
        Object.keys(collieTools(connection, { profile: "browser" })),
      );
      expect(summary).toMatchSnapshot();
    },
  );

  test(
    "offers none of the kept-off operations or granting parameters on any profile",
    { tags: ["adapter:ai-sdk", "story:11-245"] },
    async () => {
      const connection = stubConnection(async () => ({}));
      const forbiddenToolNames = new Set(
        KEPT_OFF_OPERATIONS.map((operation) => operation.replaceAll(".", "_")),
      );

      const grantingParamsFound: string[] = [];
      for (const profile of ALL_PROFILES) {
        const tools = collieTools(connection, { profile });
        for (const toolName of Object.keys(tools)) {
          expect(forbiddenToolNames.has(toolName), `${profile} must not expose ${toolName}`).toBe(
            false,
          );
          expect(toolName.startsWith("live_view_"), `${profile} must not expose ${toolName}`).toBe(
            false,
          );
        }

        for (const startName of ["run_start", "recipe_start"]) {
          const startTool = tools[startName];
          if (startTool !== undefined) {
            const schema = await asSchema(startTool.inputSchema).jsonSchema;
            if (
              schema.properties !== undefined &&
              Object.hasOwn(schema.properties, "accept_plugin_change")
            ) {
              grantingParamsFound.push(`${profile}:${startName}`);
            }
          }
        }
      }

      expect(grantingParamsFound).toEqual([]);
      expect(forbiddenToolNames.has("page_lend")).toBe(true);
      expect(forbiddenToolNames.has("plugin_add")).toBe(true);
      expect(forbiddenToolNames.has("model_set_price")).toBe(true);
      expect(COLLIE_MCP_PROFILES.browser.length).toBeGreaterThan(0);
    },
  );

  test(
    "announces form tool, never retries retryable errors, and forwards error-json with fix",
    { tags: ["adapter:ai-sdk", "story:11-245", "story:11-246"] },
    async () => {
      const calls: {
        operation: string;
        params: RawParams | undefined;
        form: string | undefined;
      }[] = [];
      const connection = stubConnection(async (operation, params, options) => {
        calls.push({ operation, params, form: options?.form });
        throw new ProgramError("selector not found", {
          code: "element_not_found",
          group: "program",
          retryable: true,
          operation,
          fix: "Check the selector or wait for the element to appear.",
        });
      });

      const tools = collieTools(connection, { profile: "browser" });
      let step = 0;
      const model = new MockLanguageModelV4({
        doGenerate: async () => {
          step += 1;
          if (step === 1) {
            return {
              content: [
                {
                  type: "tool-call",
                  toolCallId: "call-1",
                  toolName: "page_read",
                  input: JSON.stringify({ page_id: "p-1", fields: {} }),
                },
              ],
              finishReason: { unified: "tool-calls", raw: undefined },
              usage: {
                inputTokens: {
                  total: 10,
                  noCache: 10,
                  cacheRead: undefined,
                  cacheWrite: undefined,
                },
                outputTokens: { total: 5, text: undefined, reasoning: undefined },
              },
              warnings: [],
            };
          }
          return {
            content: [{ type: "text", text: "recovered" }],
            finishReason: { unified: "stop", raw: undefined },
            usage: {
              inputTokens: { total: 20, noCache: 20, cacheRead: undefined, cacheWrite: undefined },
              outputTokens: { total: 3, text: 3, reasoning: undefined },
            },
            warnings: [],
          };
        },
      });

      const result = await generateText({
        model,
        tools,
        prompt: "Read the page title",
        stopWhen: stepCountIs(5),
      });

      expect(result.text).toBe("recovered");
      expect(calls).toHaveLength(1);
      expect(calls[0]).toEqual({
        operation: "page.read",
        params: { page_id: "p-1", fields: {} },
        form: "tool",
      });

      // The second model call must receive the error-json tool result with `code` and `fix`.
      const secondPrompt = model.doGenerateCalls[1]?.prompt ?? [];
      const toolMessage = secondPrompt.find((message) => message.role === "tool");
      expect(toolMessage).toBeDefined();
      const firstPart = toolMessage?.content[0];
      expect(firstPart).toMatchObject({
        type: "tool-result",
        toolName: "page_read",
        output: {
          type: "error-json",
          value: {
            code: "element_not_found",
            group: "program",
            retryable: true,
            fix: "Check the selector or wait for the element to appear.",
          },
        },
      });
    },
  );

  test(
    "loads no AI provider or Playwright on import",
    { tags: ["adapter:ai-sdk", "story:11-245"] },
    () => {
      const entry = fileURLToPath(new URL("./index.ts", import.meta.url));
      const stdout = execFileSync(
        process.execPath,
        [
          "--input-type=module",
          "-e",
          `import ${JSON.stringify(entry)}; console.log(JSON.stringify(process.moduleLoadList));`,
        ],
        { encoding: "utf8" },
      );
      const parsed: unknown = JSON.parse(stdout);
      const loaded = Array.isArray(parsed)
        ? parsed.filter((item): item is string => typeof item === "string")
        : [];
      expect(loaded.length).toBeGreaterThan(0);
      const joined = loaded.join("\n").toLowerCase();
      for (const forbidden of [
        "@ai-sdk/openai",
        "@ai-sdk/anthropic",
        "@ai-sdk/google",
        "playwright",
      ]) {
        expect(joined, `import must not load ${forbidden}`).not.toContain(forbidden);
      }
    },
  );
});
