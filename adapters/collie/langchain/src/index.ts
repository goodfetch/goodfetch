// `@goodfetch/collie-langchain` (ADRs 0011, 0070, 0095 and 0099; spec #608):
// projects Collie's MCP tool profiles onto LangChain.js `DynamicStructuredTool`
// instances usable with both LangChain and LangGraph. Loads no AI provider or
// Playwright on import (ADR 0071).
import { isGoodfetchError, type Connection } from "@goodfetch/collie";
import {
  toRawParams,
  toToolError,
  type CollieToolError,
  type CollieToolOutput,
  type CollieToolsOptions,
} from "@goodfetch/collie-adapter-shared";
import {
  collieMcpTools,
  type McpProfile,
  type McpToolSpec,
} from "@goodfetch/collie-client/mcp-tools";
import { tool, type DynamicStructuredTool } from "@langchain/core/tools";

export type { McpOperationClass, McpProfile } from "@goodfetch/collie-client/mcp-tools";
export type { CollieToolError, CollieToolOutput, CollieToolsOptions };

function createCollieTool(
  connection: Pick<Connection, "call">,
  spec: McpToolSpec,
): DynamicStructuredTool {
  return tool(
    async (input: unknown, config): Promise<string> => {
      try {
        const result = await connection.call(spec.operation, toRawParams(input), {
          signal: config.signal,
          form: "tool",
        });
        const payload: CollieToolOutput = { ok: true, result };
        return JSON.stringify(payload);
      } catch (error) {
        if (isGoodfetchError(error)) {
          const payload: CollieToolOutput = {
            ok: false,
            error: toToolError(error, spec.operation),
          };
          return JSON.stringify(payload);
        }
        throw error;
      }
    },
    {
      name: spec.name,
      description: spec.description,
      schema: spec.inputSchema,
      metadata: {
        readOnly: spec.readOnly,
        operationClass: spec.class,
        destructive: spec.destructive,
        idempotent: spec.idempotent,
        openWorld: spec.openWorld,
      },
    },
  );
}

/**
 * Returns LangChain.js tools for Collie's chosen MCP profile (`"browser"` by
 * default), usable directly with both LangChain and LangGraph.
 *
 * Every tool delegates to `connection.call(operation, params, { form: "tool" })`,
 * preserving Collie's one-catalog contract, Error codes, `fix` hints in tool
 * form, and non-retrying semantics (ADRs 0011, 0070, 0095 and 0099).
 */
export function collieTools(
  connection: Pick<Connection, "call">,
  options?: CollieToolsOptions,
): DynamicStructuredTool[] {
  const profile: McpProfile = options?.profile ?? "browser";
  return collieMcpTools(profile).map((spec) => createCollieTool(connection, spec));
}
