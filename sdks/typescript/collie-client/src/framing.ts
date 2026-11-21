// The Connection's framing (ADRs 0024 and 0038): one JSON-RPC 2.0 message
// per line of UTF-8, each way. The engine's side is read strictly, as
// `StrictStdout` reads it in Rust's tests (#361): a line that is not exactly
// one JSON-RPC message, such as a log line on an Attached engine's stdout,
// or that is not UTF-8, is reported as invalid, never skipped. The one
// difference, a partial line at the end, is `readLines`'s to explain.

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

/** Whether `value` is a JSON object: not null, not a list. */
export function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
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

const LINE_FEED = 0x0a;
const strict = new TextDecoder("utf-8", { fatal: true });
const lossy = new TextDecoder("utf-8");

/** The line `bytes` hold, invalid when they are not UTF-8. */
function lineOf(bytes: Uint8Array): Line {
  let line: string;
  try {
    line = strict.decode(bytes);
  } catch {
    return { kind: "invalid", line: lossy.decode(bytes) };
  }
  const message = parseLine(line);
  return message === undefined ? { kind: "invalid", line } : { kind: "message", message };
}

/** The bytes of `parts` then `last`, copied once; `last` itself when there are no `parts`. */
function joined(parts: readonly Uint8Array[], last: Uint8Array): Uint8Array {
  if (parts.length === 0) {
    return last;
  }
  const length = parts.reduce((sum, part) => sum + part.length, last.length);
  const whole = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    whole.set(part, offset);
    offset += part.length;
  }
  whole.set(last, offset);
  return whole;
}

/**
 * Each whole line the engine's bytes hold, in order, until they end. Lines
 * are split on bytes, so a character split across chunks stays whole, and
 * each is decoded strictly. A partial line is kept as the chunks that hold
 * it and joined once, when its line feed arrives, and only the new chunk is
 * searched for one: reading a line costs time linear in its size, however
 * many chunks deliver it. Unlike `StrictStdout`, a partial line at the end
 * is dropped rather than reported: the engine stopped mid-write, and its
 * end, with its exit status, says more than the partial line would.
 */
export async function* readLines(chunks: AsyncIterable<Uint8Array>): AsyncGenerator<Line> {
  const partial: Uint8Array[] = [];
  for await (const bytes of chunks) {
    let start = 0;
    let end = bytes.indexOf(LINE_FEED);
    while (end !== -1) {
      yield lineOf(joined(partial, bytes.subarray(start, end)));
      partial.length = 0;
      start = end + 1;
      end = bytes.indexOf(LINE_FEED, start);
    }
    if (start < bytes.length) {
      partial.push(bytes.subarray(start));
    }
  }
}

const encoder = new TextEncoder();

/** The bytes of one message, its line ended. */
export function encodeMessage(message: Readonly<Record<string, unknown>>): Uint8Array {
  return encoder.encode(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
}
