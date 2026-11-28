// `@goodfetch/collie-ai-sdk` (ADRs 0011, 0070, 0095 and 0099; spec #608):
// projects Collie's MCP tool profiles onto Vercel AI SDK `tool()` objects.
// Loads no AI provider or Playwright on import (ADR 0071).
import { isGoodfetchError, type Connection, type RawParams } from "@goodfetch/collie";
import {
  errorToJsonObject,
  toJsonValue,
  toToolError,
  type CollieToolError,
  type CollieToolOutput,
} from "../../shared/index.ts";
import {
  collieMcpTools,
  type McpProfile,
  type McpToolSpec,
} from "@goodfetch/collie-client/mcp-tools";
import { jsonSchema, tool, type Tool } from "ai";

export type { McpOperationClass, McpProfile } from "@goodfetch/collie-client/mcp-tools";
export type { CollieToolError, CollieToolOutput };

/** Options for `collieTools`. */
export interface CollieToolsOptions {
  /** Which Collie MCP tool profile to project; defaults to `"browser"`. */
  readonly profile?: McpProfile | undefined;
}

/** Map of MCP tool names to executable Vercel AI SDK tools. */
export type CollieToolSet = Record<string, Tool<RawParams, CollieToolOutput>>;

function createCollieTool(
  connection: Pick<Connection, "call">,
  spec: McpToolSpec,
): Tool<RawParams, CollieToolOutput> {
  return tool({
    description: spec.description,
    inputSchema: jsonSchema<RawParams>(spec.inputSchema),
    metadata: {
      readOnly: spec.readOnly,
      operationClass: spec.class,
      destructive: spec.destructive,
      idempotent: spec.idempotent,
      openWorld: spec.openWorld,
    },
    execute: async (input, options): Promise<CollieToolOutput> => {
      try {
        const result = await connection.call(spec.operation, input, {
          signal: options.abortSignal,
          form: "tool",
        });
        return { ok: true, result };
      } catch (error) {
        if (isGoodfetchError(error)) {
          return { ok: false, error: toToolError(error, spec.operation) };
        }
        throw error;
      }
    },
    toModelOutput: ({ output }) => {
      if (!output.ok) {
        return { type: "error-json", value: errorToJsonObject(output.error) };
      }
      return { type: "json", value: toJsonValue(output.result) };
    },
  });
}

/**
 * Returns Collie's operations for `profile` (default `"browser"`) as Vercel AI
 * SDK tool objects backed by `connection`.
 *
 * Each tool uses its canonical MCP projection name, description, input schema
 * and read-only metadata, announces form `"tool"`, never retries, and turns
 * Goodfetch Error codes into AI SDK `error-json` tool results carrying `fix`.
 */
export function collieTools(
  connection: Pick<Connection, "call">,
  options: CollieToolsOptions = {},
): CollieToolSet {
  const profile = options.profile ?? "browser";
  const tools: CollieToolSet = {};
  for (const spec of collieMcpTools(profile)) {
    tools[spec.name] = createCollieTool(connection, spec);
  }
  return tools;
}
