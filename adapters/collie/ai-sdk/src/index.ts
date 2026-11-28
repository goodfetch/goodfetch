// `@goodfetch/collie-ai-sdk` (ADRs 0011, 0070, 0095 and 0099; spec #608):
// projects Collie's MCP tool profiles onto Vercel AI SDK `tool()` objects.
// Loads no AI provider or Playwright on import (ADR 0071).
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
import { jsonSchema, tool, type JSONValue as AiJsonValue, type Tool } from "ai";

type AiJsonObject = { [key: string]: AiJsonValue };

export type { McpOperationClass, McpProfile } from "@goodfetch/collie-client/mcp-tools";

/** Options for {@link collieTools}. */
export interface CollieToolsOptions {
  /** Which Collie MCP tool profile to project; defaults to `"browser"`. */
  readonly profile?: McpProfile | undefined;
}

/** Structured Goodfetch Error code returned as a tool error to the model. */
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
  readonly limit?: AiJsonValue | undefined;
  readonly details?: Readonly<Record<string, AiJsonValue>> | undefined;
}

/** Discriminated tool execution outcome before {@link toModelOutput} conversion. */
export type CollieToolOutput =
  | { readonly ok: true; readonly result: JSONValue }
  | { readonly ok: false; readonly error: CollieToolError };

/** Map of MCP tool names to executable Vercel AI SDK tools. */
export type CollieToolSet = Record<string, Tool<RawParams, CollieToolOutput>>;

function toAiJsonObject(record: Readonly<Record<string, unknown>>): AiJsonObject {
  const out: AiJsonObject = {};
  for (const [key, entry] of Object.entries(record)) {
    if (entry !== undefined) {
      out[key] = toAiJsonValue(entry);
    }
  }
  return out;
}

function toAiJsonValue(value: unknown): AiJsonValue {
  if (
    value === null ||
    typeof value === "string" ||
    typeof value === "number" ||
    typeof value === "boolean"
  ) {
    return value;
  }
  if (Array.isArray(value)) {
    return value.map((item) => toAiJsonValue(item));
  }
  if (typeof value === "object") {
    return toAiJsonObject(Object.fromEntries(Object.entries(value)));
  }
  return typeof value === "bigint" ? value.toString() : "";
}

function toToolError(error: GoodfetchError, fallbackOperation: string): CollieToolError {
  const details = Object.keys(error.details).length > 0 ? toAiJsonObject(error.details) : undefined;
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
    ...(error.limit === undefined ? {} : { limit: toAiJsonValue(error.limit) }),
    ...(details === undefined ? {} : { details }),
  };
}

function errorToJsonObject(error: CollieToolError): AiJsonObject {
  const out: AiJsonObject = {
    code: error.code,
    group: error.group,
    retryable: error.retryable,
    message: error.message,
  };
  if (error.operation !== undefined) {
    out["operation"] = error.operation;
  }
  if (error.fix !== undefined) {
    out["fix"] = error.fix;
  }
  if (error.target !== undefined) {
    out["target"] = error.target;
  }
  if (error.url !== undefined) {
    out["url"] = error.url;
  }
  if (error.observed !== undefined) {
    out["observed"] = error.observed;
  }
  if (error.limit !== undefined) {
    out["limit"] = toAiJsonValue(error.limit);
  }
  if (error.details !== undefined) {
    out["details"] = toAiJsonValue(error.details);
  }
  return out;
}

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
      return { type: "json", value: toAiJsonValue(output.result) };
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
