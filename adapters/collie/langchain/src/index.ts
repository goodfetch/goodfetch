// `@goodfetch/collie-langchain` (ADRs 0011, 0070, 0095 and 0099; spec #608):
// projects Collie's MCP tool profiles onto LangChain.js `DynamicStructuredTool`
// instances usable with both LangChain and LangGraph. Loads no AI provider or
// Playwright on import (ADR 0071).
import {
  isGoodfetchError,
  type Connection,
  type GoodfetchError,
  type JSONValue,
  type RawParams,
} from "@goodfetch/collie";
import {
  collieMcpTools,
  type McpProfile,
  type McpToolSpec,
} from "@goodfetch/collie-client/mcp-tools";
import { tool, type DynamicStructuredTool } from "@langchain/core/tools";

export type { McpOperationClass, McpProfile } from "@goodfetch/collie-client/mcp-tools";

/** Options for {@link collieTools}. */
export interface CollieToolsOptions {
  /** Which Collie MCP tool profile to project; defaults to `"browser"`. */
  readonly profile?: McpProfile | undefined;
}

/** Structured Goodfetch Error returned as JSON to the model on tool failure. */
export interface CollieToolError {
  readonly code: string;
  readonly group: string;
  readonly retryable: boolean;
  readonly message: string;
  readonly operation?: string | undefined;
  readonly fix?: string | undefined;
  readonly target?: string | undefined;
  readonly url?: string | undefined;
  readonly observed?: string | undefined;
  readonly limit?: JSONValue | undefined;
  readonly details?: Readonly<Record<string, JSONValue>> | undefined;
}

/** Discriminated tool execution outcome serialized for LangChain / LangGraph. */
export type CollieToolOutput =
  | { readonly ok: true; readonly result: JSONValue }
  | { readonly ok: false; readonly error: CollieToolError };

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function toJsonValue(value: unknown): JSONValue {
  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "number" ||
    typeof value === "boolean"
  ) {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => toJsonValue(item));
  }
  if (typeof value === "object") {
    const out: Record<string, JSONValue> = {};
    for (const [key, entry] of Object.entries(value)) {
      if (entry !== undefined) {
        out[key] = toJsonValue(entry);
      }
    }
    return out;
  }
  return typeof value === "bigint" ? value.toString() : "";
}

function toRawParams(input: unknown): RawParams {
  if (!isRecord(input)) {
    return {};
  }
  const out: Record<string, JSONValue> = {};
  for (const [key, entry] of Object.entries(input)) {
    if (entry !== undefined) {
      out[key] = toJsonValue(entry);
    }
  }
  return out;
}

function toToolError(error: GoodfetchError, fallbackOperation: string): CollieToolError {
  const detailEntries = Object.entries(error.details);
  let details: Record<string, JSONValue> | undefined;
  if (detailEntries.length > 0) {
    details = {};
    for (const [key, entry] of detailEntries) {
      if (entry !== undefined) {
        details[key] = toJsonValue(entry);
      }
    }
  }
  return {
    code: error.code,
    group: error.group,
    retryable: error.retryable,
    message: error.message,
    operation: error.operation ?? fallbackOperation,
    ...(error.fix === undefined ? {} : { fix: error.fix }),
    ...(error.target === undefined ? {} : { target: error.target }),
    ...(error.url === undefined ? {} : { url: error.url }),
    ...(error.observed === undefined ? {} : { observed: error.observed }),
    ...(error.limit === undefined ? {} : { limit: toJsonValue(error.limit) }),
    ...(details === undefined ? {} : { details }),
  };
}

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
