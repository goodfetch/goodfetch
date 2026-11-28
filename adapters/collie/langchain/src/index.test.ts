// Unit tests for `@goodfetch/collie-langchain` (ADRs 0011, 0070, 0071, 0095,
// 0099; spec #608, issue 1380): converted tool snapshots, kept-off surface
// check, form `tool` + no-retry check, and clean-process import check.
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
import { convertToOpenAITool } from "@langchain/core/utils/function_calling";
import { describe, expect, test } from "vitest";
import { collieTools } from "./index.ts";

const ALL_PROFILES: readonly McpProfile[] = ["browser", "agent", "run", "code", "full"];

function stubConnection(
  onCall: (operation: string, params?: RawParams, options?: RawCallOptions) => Promise<JSONValue>,
): Pick<Connection, "call"> {
  return { call: onCall };
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

describe("@goodfetch/collie-langchain", () => {
  test(
    "snapshots the converted LangChain tools per profile and preserves metadata",
    { tags: ["adapter:langchain", "story:11-242"] },
    () => {
      const connection = stubConnection(async () => ({}));
      const summary: Record<
        string,
        readonly {
          name: string;
          description: string;
          readOnly: unknown;
          operationClass: unknown;
          hasObjectSchema: boolean;
        }[]
      > = {};

      for (const profile of ALL_PROFILES) {
        const tools = collieTools(connection, { profile });
        summary[profile] = tools.map((definition) => {
          const openaiTool = convertToOpenAITool(definition);
          const params: unknown = openaiTool.function.parameters;
          return {
            name: openaiTool.function.name,
            description: openaiTool.function.description ?? "",
            readOnly: definition.metadata?.["readOnly"],
            operationClass: definition.metadata?.["operationClass"],
            hasObjectSchema: isRecord(params) && params["type"] === "object",
          };
        });
      }

      expect(collieTools(connection).map((item) => item.name)).toEqual(
        collieTools(connection, { profile: "browser" }).map((item) => item.name),
      );
      expect(summary).toMatchSnapshot();
    },
  );

  test(
    "offers none of the kept-off operations or granting parameters on any profile",
    { tags: ["adapter:langchain", "story:11-242"] },
    () => {
      const connection = stubConnection(async () => ({}));
      const forbiddenToolNames = new Set(
        KEPT_OFF_OPERATIONS.map((operation) => operation.replaceAll(".", "_")),
      );

      const grantingParamsFound: string[] = [];
      for (const profile of ALL_PROFILES) {
        const tools = collieTools(connection, { profile });
        for (const item of tools) {
          expect(forbiddenToolNames.has(item.name), `${profile} must not expose ${item.name}`).toBe(
            false,
          );
          expect(
            item.name.startsWith("live_view_"),
            `${profile} must not expose ${item.name}`,
          ).toBe(false);
          if (item.name === "run_start" || item.name === "recipe_start") {
            const converted = convertToOpenAITool(item);
            const params: unknown = converted.function.parameters;
            if (
              isRecord(params) &&
              isRecord(params["properties"]) &&
              Object.hasOwn(params["properties"], "accept_plugin_change")
            ) {
              grantingParamsFound.push(`${profile}:${item.name}`);
            }
          }
        }
      }

      expect(grantingParamsFound).toEqual([]);
      expect(forbiddenToolNames.has("page_lend")).toBe(true);
      expect(COLLIE_MCP_PROFILES.browser.length).toBeGreaterThan(0);
    },
  );

  test(
    "announces form tool, never retries retryable errors, and returns structured error JSON",
    { tags: ["adapter:langchain", "story:11-242"] },
    async () => {
      const calls: {
        operation: string;
        params: RawParams | undefined;
        form: string | undefined;
      }[] = [];

      const connection = stubConnection(async (operation, params, options) => {
        calls.push({ operation, params, form: options?.form });
        if (operation === "page.goto") {
          throw new ProgramError("invalid setting", {
            code: "setting_not_applicable",
            group: "program",
            retryable: true,
            operation,
            fix: "Remove `mcp.oauth.issuer` from the run settings.",
          });
        }
        return { run_id: "r1" };
      });

      const tools = collieTools(connection, { profile: "browser" });
      const runStart = tools.find((item) => item.name === "run_start");
      const pageGoto = tools.find((item) => item.name === "page_goto");
      expect(runStart).toBeDefined();
      expect(pageGoto).toBeDefined();

      const okRaw = await runStart?.invoke({});
      const okParsed: unknown = typeof okRaw === "string" ? JSON.parse(okRaw) : okRaw;
      expect(okParsed).toEqual({ ok: true, result: { run_id: "r1" } });

      const errRaw = await pageGoto?.invoke({ page_id: "p1", url: "https://example.test" });
      const errParsed: unknown = typeof errRaw === "string" ? JSON.parse(errRaw) : errRaw;
      expect(errParsed).toMatchObject({
        ok: false,
        error: {
          code: "setting_not_applicable",
          group: "program",
          retryable: true,
          fix: "Remove `mcp.oauth.issuer` from the run settings.",
        },
      });
      expect(calls).toEqual([
        { operation: "run.start", params: {}, form: "tool" },
        {
          operation: "page.goto",
          params: { page_id: "p1", url: "https://example.test" },
          form: "tool",
        },
      ]);
    },
  );

  test(
    "loads no AI provider or Playwright on import in a clean Node process",
    { tags: ["adapter:langchain", "story:11-242"] },
    () => {
      const entryUrl = new URL("./index.ts", import.meta.url).href;
      const script = [
        `const mod = await import(${JSON.stringify(entryUrl)});`,
        "const loaded = [...process.moduleLoadList];",
        "console.log(JSON.stringify({ exports: Object.keys(mod).sort(), loaded }));",
      ].join("\n");
      const stdout = execFileSync(process.execPath, ["--input-type=module", "-e", script], {
        cwd: fileURLToPath(new URL("..", import.meta.url)),
        encoding: "utf8",
      });
      const parsed: unknown = JSON.parse(stdout);
      expect(isRecord(parsed)).toBe(true);
      const loaded = isRecord(parsed) && Array.isArray(parsed["loaded"]) ? parsed["loaded"] : [];
      const forbidden = loaded.filter(
        (entry) =>
          typeof entry === "string" &&
          /playwright|@langchain\/openai|@langchain\/anthropic|@langchain\/google|openai|anthropic/i.test(
            entry,
          ),
      );
      expect(forbidden).toEqual([]);
    },
  );
});
