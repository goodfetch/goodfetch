// Unit tests for `@goodfetch/collie-mastra` (ADRs 0011, 0070, 0071, 0095,
// 0099; spec #608, issue 1381): converted tool snapshots, kept-off surface
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
import { standardSchemaToJSONSchema } from "@mastra/core/schema";
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

describe("@goodfetch/collie-mastra", () => {
  test(
    "snapshots the converted Mastra tools per profile and maps MCP annotations",
    { tags: ["adapter:mastra", "story:11-243"] },
    () => {
      const connection = stubConnection(async () => ({}));
      const summary: Record<
        string,
        readonly {
          id: string;
          description: string;
          readOnlyHint: boolean | undefined;
          destructiveHint: boolean | undefined;
          idempotentHint: boolean | undefined;
          openWorldHint: boolean | undefined;
          hasObjectSchema: boolean;
        }[]
      > = {};

      for (const profile of ALL_PROFILES) {
        const tools = collieTools(connection, { profile });
        summary[profile] = Object.entries(tools).map(([name, definition]) => {
          const schema: unknown =
            definition.inputSchema === undefined
              ? undefined
              : standardSchemaToJSONSchema(definition.inputSchema);
          return {
            id: name,
            description: definition.description,
            readOnlyHint: definition.mcp?.annotations?.readOnlyHint,
            destructiveHint: definition.mcp?.annotations?.destructiveHint,
            idempotentHint: definition.mcp?.annotations?.idempotentHint,
            openWorldHint: definition.mcp?.annotations?.openWorldHint,
            hasObjectSchema: isRecord(schema) && schema["type"] === "object",
          };
        });
      }

      expect(Object.keys(collieTools(connection))).toEqual(
        Object.keys(collieTools(connection, { profile: "browser" })),
      );
      expect(summary).toMatchSnapshot();
    },
  );

  test(
    "offers none of the kept-off operations or granting parameters on any profile",
    { tags: ["adapter:mastra", "story:11-243"] },
    () => {
      const connection = stubConnection(async () => ({}));
      const forbiddenToolNames = new Set(
        KEPT_OFF_OPERATIONS.map((operation) => operation.replaceAll(".", "_")),
      );

      const grantingParamsFound: string[] = [];
      for (const profile of ALL_PROFILES) {
        const tools = collieTools(connection, { profile });
        for (const [toolName, definition] of Object.entries(tools)) {
          expect(forbiddenToolNames.has(toolName), `${profile} must not expose ${toolName}`).toBe(
            false,
          );
          expect(toolName.startsWith("live_view_"), `${profile} must not expose ${toolName}`).toBe(
            false,
          );
          if (
            (toolName === "run_start" || toolName === "recipe_start") &&
            definition.inputSchema !== undefined
          ) {
            const schema: unknown = standardSchemaToJSONSchema(definition.inputSchema);
            if (
              isRecord(schema) &&
              isRecord(schema["properties"]) &&
              Object.hasOwn(schema["properties"], "accept_plugin_change")
            ) {
              grantingParamsFound.push(`${profile}:${toolName}`);
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
    "announces form tool, never retries retryable errors, and returns structured error with fix",
    { tags: ["adapter:mastra", "story:11-243"] },
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
      const runStart = tools["run_start"];
      const pageGoto = tools["page_goto"];
      expect(runStart?.execute).toBeDefined();
      expect(pageGoto?.execute).toBeDefined();

      const okOut = await runStart?.execute?.({}, {});
      expect(okOut).toEqual({ ok: true, result: { run_id: "r1" } });

      const errOut = await pageGoto?.execute?.({ page_id: "p1", url: "https://example.test" }, {});
      expect(errOut).toMatchObject({
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
    { tags: ["adapter:mastra", "story:11-243"] },
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
          /playwright|@ai-sdk\/openai|@ai-sdk\/anthropic|@ai-sdk\/google|openai|anthropic/i.test(
            entry,
          ),
      );
      expect(forbidden).toEqual([]);
    },
  );
});
