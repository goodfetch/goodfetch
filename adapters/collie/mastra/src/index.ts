// `@goodfetch/collie-mastra` (ADRs 0011, 0070, 0095 and 0099; spec #608):
// projects Collie's MCP tool profiles onto Mastra `createTool()` definitions.
// Loads no AI provider or Playwright on import (ADR 0071).
import { isGoodfetchError, type Connection, type RawParams } from "@goodfetch/collie";
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
import { createTool, type Tool } from "@mastra/core/tools";

export type { McpOperationClass, McpProfile } from "@goodfetch/collie-client/mcp-tools";
export type { CollieToolError, CollieToolOutput, CollieToolsOptions };

/** Map of MCP tool names to executable Mastra tools. */
export type CollieToolSet = Record<string, Tool<RawParams>>;

function standardJsonSchema(schema: Readonly<Record<string, unknown>>) {
  const copy = (): Record<string, unknown> => ({ ...schema });
  return {
    "~standard": {
      version: 1 as const,
      vendor: "goodfetch",
      validate: (value: unknown) => ({ value: toRawParams(value) }),
      jsonSchema: {
        input: copy,
        output: copy,
      },
    },
  };
}

function createCollieTool(
  connection: Pick<Connection, "call">,
  spec: McpToolSpec,
): Tool<RawParams> {
  return createTool({
    id: spec.name,
    description: spec.description,
    inputSchema: standardJsonSchema(spec.inputSchema),
    mcp: {
      annotations: {
        readOnlyHint: spec.readOnly,
        destructiveHint: spec.destructive,
        idempotentHint: spec.idempotent,
        openWorldHint: spec.openWorld,
      },
    },
    execute: async (input, context): Promise<CollieToolOutput> => {
      try {
        const result = await connection.call(spec.operation, toRawParams(input), {
          signal: context.abortSignal,
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
  });
}

/**
 * Returns a Mastra tool map for Collie's chosen MCP profile (`"browser"` by
 * default).
 *
 * Every tool delegates to `connection.call(operation, params, { form: "tool" })`,
 * preserving Collie's one-catalog contract, Error codes, `fix` hints in tool
 * form, and non-retrying semantics (ADRs 0011, 0070, 0095 and 0099).
 */
export function collieTools(
  connection: Pick<Connection, "call">,
  options?: CollieToolsOptions,
): CollieToolSet {
  const profile: McpProfile = options?.profile ?? "browser";
  const tools: CollieToolSet = {};
  for (const spec of collieMcpTools(profile)) {
    tools[spec.name] = createCollieTool(connection, spec);
  }
  return tools;
}
