// The Connection's framing (ADRs 0024 and 0038): one JSON-RPC 2.0 message
// per line of UTF-8, each way. The engine's side is read strictly, as
// `StrictStdout` reads it in Rust's tests (#361): a line that is not exactly
// one JSON-RPC message, such as a log line on an Attached engine's stdout,
// is reported as invalid, never skipped.

/** A JSON-RPC request or reply id. */
export type MessageId = number | string;

/** One message the engine sent. */
export type Message =
  | {
      readonly kind: "request";
      readonly id: MessageId;
      readonly method: string;
      readonly params: unknown;
    }
  | { readonly kind: "notification"; readonly method: string; readonly params: unknown }
  | { readonly kind: "result"; readonly id: MessageId | null; readonly result: unknown }
  | { readonly kind: "error"; readonly id: MessageId | null; readonly error: unknown };

/** One line the engine sent: a message, or what is none. */
export type Line =
  | { readonly kind: "message"; readonly message: Message }
  | { readonly kind: "invalid"; readonly line: string };

const MEMBERS = new Set(["jsonrpc", "id", "method", "params", "result", "error"]);

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isId(value: unknown): value is MessageId {
  return typeof value === "string" || typeof value === "number";
}

/** The message `line` holds, or `undefined` when it is not exactly one JSON-RPC 2.0 message. */
export function parseLine(line: string): Message | undefined {
  if (line.trim() !== line || line === "") {
    return undefined;
  }
  let value: unknown;
  try {
    value = JSON.parse(line);
  } catch {
    return undefined;
  }
  if (!isRecord(value) || value["jsonrpc"] !== "2.0") {
    return undefined;
  }
  if (Object.keys(value).some((key) => !MEMBERS.has(key))) {
    return undefined;
  }
  const { id, method, params, result, error } = value;
  if (method !== undefined) {
    const hasParams = params === undefined || typeof params === "object";
    if (typeof method !== "string" || !hasParams || "result" in value || "error" in value) {
      return undefined;
    }
    if (id === undefined) {
      return { kind: "notification", method, params };
    }
    return isId(id) ? { kind: "request", id, method, params } : undefined;
  }
  if (!(isId(id) || id === null) || "result" in value === "error" in value) {
    return undefined;
  }
  if ("result" in value) {
    return { kind: "result", id, result };
  }
  const isError =
    isRecord(error) && typeof error["code"] === "number" && typeof error["message"] === "string";
  return isError ? { kind: "error", id, error } : undefined;
}

/**
 * Each whole line the engine's bytes hold, in order, until they end. A
 * partial line at the end is dropped: the engine stopped mid-write, which
 * its end reports.
 */
export async function* readLines(readable: ReadableStream<Uint8Array>): AsyncGenerator<Line> {
  const decoder = new TextDecoder("utf-8", { fatal: false });
  let pending = "";
  for await (const bytes of readable) {
    pending += decoder.decode(bytes, { stream: true });
    let end = pending.indexOf("\n");
    while (end !== -1) {
      const line = pending.slice(0, end);
      pending = pending.slice(end + 1);
      const message = parseLine(line);
      yield message === undefined ? { kind: "invalid", line } : { kind: "message", message };
      end = pending.indexOf("\n");
    }
  }
}

const encoder = new TextEncoder();

/** The bytes of one message, its line ended. */
export function encodeMessage(message: Readonly<Record<string, unknown>>): Uint8Array {
  return encoder.encode(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
}
