// The Scripted engine's in-process player (ADR 0069): plays one script of
// the shared corpus, `test-support/fixtures/engine-scripts/`, to a Client over
// an in-memory transport, as `goodfetch-scripted-engine` plays it over stdio,
// so an SDK's unit test runs in the Quick suite with no process. It matches
// each message the Client sends on its method and the canonical JSON (RFC
// 8785) of its params, answers with the ID of the request the step before
// matched, and fails on a message no step expects and on a step left unused.
import { readFileSync } from "node:fs";
import { TestSupportError } from "./errors.ts";
import { ajv } from "./schemas.ts";

/**
 * The value an SDK takes as its connection to the engine: the engine's bytes
 * to read, and the Client's bytes to write, newline-delimited JSON-RPC each way
 * as on an Attached engine's stdio. Closing `writable` closes the Client's end.
 */
export type InMemoryTransport = ReadableWritablePair<Uint8Array, Uint8Array>;

/** A message the play read or wrote, as `goodfetch-scripted-engine` records it. */
export type PlayedMessage =
  | { readonly from: "client" | "engine"; readonly message: unknown }
  /** A line that is not JSON, or a `raw` step's text. */
  | { readonly from: "client" | "engine"; readonly raw: string };

/** How a script that played to its end ended. */
export type Ending =
  /** The Client closed its end after the last step, or a `stall`. */
  | { readonly kind: "finished" }
  /** The script closed the connection. */
  | { readonly kind: "closed" }
  /** The script's engine exited at once with this code, as a crash does. */
  | { readonly kind: "exit"; readonly code: number };

/** A script being played. */
export interface PlayedScript {
  /** The transport to hand the SDK. */
  readonly transport: InMemoryTransport;
  /** Every message read and written so far, in order. */
  readonly messages: readonly PlayedMessage[];
  /**
   * Ends the play as the Client closing its end would, unless it has ended,
   * and says how the script ended.
   *
   * @throws {TestSupportError} `unexpected_message` when the Client sent a
   * message no step expects; `unused_step` when a step was left unused;
   * `nothing_to_answer` when a `reply` or `await_reply` follows no request.
   */
  done(): Ending;
}

type Call = { readonly method: string; readonly params?: unknown };
type Reply = { readonly result: unknown } | { readonly error: unknown };

/** One step of a script; `script.schema.json` is the contract. */
type Step =
  | { readonly expect: Call }
  | { readonly reply: Reply }
  | { readonly send: { readonly request: Call } | { readonly notification: Call } }
  | { readonly await_reply: Reply }
  | { readonly stall: object }
  | { readonly close: object }
  | { readonly raw: { readonly text: string } }
  | { readonly exit: { readonly code: number } };

type Script = { readonly description: string; readonly steps: readonly Step[] };

const corpus = new URL("../../fixtures/engine-scripts/", import.meta.url);

const isScript = ajv.compile<Script>(
  JSON.parse(readFileSync(new URL("script.schema.json", corpus), "utf8")),
);

/**
 * Plays the corpus's script `script`, such as `"self-test/call-answered.json"`,
 * or the script file at a URL, to the Client that takes `transport`.
 *
 * @throws {TestSupportError} `fixture_invalid` when the file fails the
 * committed `script.schema.json`.
 */
export function playScript(script: string | URL): PlayedScript {
  const url = typeof script === "string" ? new URL(script, corpus) : script;
  const value: unknown = JSON.parse(readFileSync(url, "utf8"));
  if (!isScript(value)) {
    throw new TestSupportError(
      "fixture_invalid",
      `the script ${url.pathname} fails script.schema.json: ${ajv.errorsText(isScript.errors)}`,
    );
  }
  return new Player(value.steps);
}

const encoder = new TextEncoder();

/** The state of one play: the step it is at, and what it answers next. */
class Player implements PlayedScript {
  readonly transport: InMemoryTransport;
  readonly messages: PlayedMessage[] = [];
  readonly #steps: readonly Step[];
  readonly #engine: ReadableStreamDefaultController<Uint8Array>;
  readonly #decoder = new TextDecoder();
  #buffered = "";
  #index = 0;
  /** The ID of the request the last `expect` matched, which `reply` answers. */
  #answering: { readonly id: unknown } | undefined;
  /** The ID of the request the last `send` sent, which `await_reply` matches. */
  #awaiting: number | undefined;
  #nextId = 1;
  #outcome: Ending | TestSupportError | undefined;

  constructor(steps: readonly Step[]) {
    this.#steps = steps;
    let engine: ReadableStreamDefaultController<Uint8Array> | undefined;
    const readable = new ReadableStream<Uint8Array>({
      start: (controller) => {
        engine = controller;
      },
    });
    if (engine === undefined) throw new Error("invariant: a stream starts as it is built");
    this.#engine = engine;
    const writable = new WritableStream<Uint8Array>({
      write: (chunk) => {
        this.#buffered += this.#decoder.decode(chunk, { stream: true });
        let end = this.#buffered.indexOf("\n");
        while (end !== -1) {
          const line = this.#buffered.slice(0, end);
          this.#buffered = this.#buffered.slice(end + 1);
          this.#read(line);
          end = this.#buffered.indexOf("\n");
        }
        this.#throwFailure();
      },
      close: () => {
        this.#clientClosed();
        this.#throwFailure();
      },
      abort: () => {
        this.#clientClosed();
      },
    });
    this.transport = { readable, writable };
    this.#advance();
  }

  done(): Ending {
    this.#clientClosed();
    const outcome = this.#outcome;
    if (outcome instanceof TestSupportError || outcome === undefined) {
      throw outcome ?? new Error("invariant: a closed play has an outcome");
    }
    return outcome;
  }

  /** Plays the steps the engine starts, up to one that waits for the Client. */
  #advance(): void {
    while (this.#outcome === undefined && this.#index < this.#steps.length) {
      const index = this.#index;
      const step = this.#steps[index];
      if (step === undefined) throw new Error("invariant: the index is within the steps");
      // As the executable does, an `await_reply` with no request sent
      // before it fails at once.
      if ("await_reply" in step && this.#awaiting === undefined) {
        return this.#fail(nothingToAnswer(index));
      }
      if ("expect" in step || "await_reply" in step || "stall" in step) return;
      this.#index += 1;
      if ("reply" in step) {
        const answering = this.#answering;
        this.#answering = undefined;
        if (answering === undefined) return this.#fail(nothingToAnswer(index));
        this.#write({ jsonrpc: "2.0", id: answering.id, ...step.reply });
      } else if ("send" in step) {
        if ("request" in step.send) {
          const id = this.#nextId++;
          this.#write({ jsonrpc: "2.0", id, ...step.send.request });
          this.#awaiting = id;
        } else {
          this.#write({ jsonrpc: "2.0", ...step.send.notification });
        }
      } else if ("raw" in step) {
        this.#engine.enqueue(encoder.encode(step.raw.text));
        this.messages.push({ from: "engine", raw: step.raw.text });
      } else {
        this.#end("close" in step ? { kind: "closed" } : { kind: "exit", code: step.exit.code });
      }
    }
  }

  /** Takes one line the Client wrote. */
  #read(line: string): void {
    if (this.#outcome !== undefined) return;
    const parsed = parse(line);
    this.messages.push(
      parsed === undefined
        ? { from: "client", raw: line }
        : { from: "client", message: parsed.value },
    );
    // The real engine refuses what is not one JSON-RPC 2.0 message: a
    // missing or other `jsonrpc`, a reply with both `result` and `error`.
    const message =
      parsed !== undefined && isObject(parsed.value) && isJsonRpc(parsed.value)
        ? parsed.value
        : undefined;
    const index = this.#index;
    const step = this.#steps[index];
    if (step === undefined || "stall" in step) {
      const expected =
        step === undefined
          ? "the Client to close its end after the last step"
          : "no message until the Client gives up";
      return this.#fail(unexpected(undefined, expected, message, line));
    }
    if (message === undefined) {
      return this.#fail(unexpected(index, "a JSON-RPC 2.0 message", message, line));
    }
    if ("expect" in step) {
      const wanted = step.expect;
      const matches =
        !("result" in message) &&
        !("error" in message) &&
        message["method"] === wanted.method &&
        canonical(message["params"]) === canonical(wanted.params);
      if (!matches) {
        const expected = `\`${wanted.method}\` with params ${canonical(wanted.params)}`;
        return this.#fail(unexpected(index, expected, message, line));
      }
      // A notification, such as a `$/cancel`, leaves the request matched
      // before it to answer.
      if ("id" in message) this.#answering = { id: message["id"] };
    } else if ("await_reply" in step) {
      const id = this.#awaiting;
      this.#awaiting = undefined;
      if (id === undefined) throw new Error("invariant: #advance stops at an awaited reply only");
      const [key, value] = Object.entries(step.await_reply)[0] ?? [];
      if (key === undefined) throw new Error("invariant: a reply holds a result or an error");
      const matches =
        !("method" in message) &&
        message["id"] === id &&
        canonical(message[key]) === canonical(value);
      if (!matches) {
        const expected = `the answer {"${key}": ${canonical(value)}} to request ${id}`;
        return this.#fail(unexpected(index, expected, message, line));
      }
    } else {
      throw new Error(
        "invariant: the play waits for the Client only at expect, await_reply or stall",
      );
    }
    this.#index += 1;
    this.#advance();
  }

  /** The Client closed its end: a `stall`, or the end of the script, finishes the play. */
  #clientClosed(): void {
    if (this.#outcome !== undefined) return;
    if (this.#buffered !== "") {
      const line = this.#buffered;
      this.#buffered = "";
      this.#read(line);
      if (this.#outcome !== undefined) return;
    }
    const step = this.#steps[this.#index];
    if (step !== undefined && "stall" in step) this.#index += 1;
    if (this.#index < this.#steps.length) return this.#fail(unusedStep(this.#index));
    this.#end({ kind: "finished" });
  }

  #write(message: object): void {
    this.#engine.enqueue(encoder.encode(`${JSON.stringify(message)}\n`));
    this.messages.push({ from: "engine", message });
  }

  /** Ends the play, closing the engine's end; a step after the ending one is unused. */
  #end(ending: Ending): void {
    if (this.#index < this.#steps.length) return this.#fail(unusedStep(this.#index));
    this.#outcome = ending;
    this.#engine.close();
  }

  /** Fails the play: the Client's next read and write reject with `failure`, and so does `done()`. */
  #fail(failure: TestSupportError): void {
    this.#outcome = failure;
    this.#engine.error(failure);
  }

  #throwFailure(): void {
    if (this.#outcome instanceof TestSupportError) throw this.#outcome;
  }
}

/** A line's JSON value, or `undefined` when it is not JSON. */
function parse(line: string): { readonly value: unknown } | undefined {
  try {
    return { value: JSON.parse(line) };
  } catch {
    return undefined;
  }
}

/** Whether `id` is a JSON-RPC request's ID: a string or a number. */
function isId(id: unknown): boolean {
  return typeof id === "string" || typeof id === "number";
}

/**
 * Whether `message` is one JSON-RPC 2.0 message, as the executable's player
 * judges it: a request or notification (`method`, an optional string or
 * number `id`, object or array `params`) or a reply (`id` and exactly one of
 * `result` and an `error` object), with no other member.
 */
function isJsonRpc(message: Record<string, unknown>): boolean {
  const only = (members: readonly string[]): boolean =>
    Object.keys(message).every((key) => members.includes(key));
  if (message["jsonrpc"] !== "2.0") return false;
  if ("method" in message) {
    return (
      typeof message["method"] === "string" &&
      (!("id" in message) || isId(message["id"])) &&
      (!("params" in message) || isObject(message["params"]) || Array.isArray(message["params"])) &&
      only(["jsonrpc", "method", "id", "params"])
    );
  }
  const error = message["error"];
  const outcome =
    "result" in message
      ? !("error" in message)
      : isObject(error) &&
        Number.isInteger(error["code"]) &&
        typeof error["message"] === "string" &&
        Object.keys(error).every((key) => ["code", "message", "data"].includes(key));
  return (
    "id" in message &&
    (isId(message["id"]) || message["id"] === null) &&
    outcome &&
    only(["jsonrpc", "id", "result", "error"])
  );
}

function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * The canonical JSON of a value (RFC 8785), or `absent`: keys sorted by their
 * UTF-16 code units, numbers and strings as ECMAScript's `JSON.stringify`
 * writes them, which is how RFC 8785 defines both.
 */
function canonical(value: unknown): string {
  if (value === undefined) return "absent";
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (isObject(value)) {
    const entries = Object.entries(value).toSorted(([left], [right]) =>
      left < right ? -1 : left > right ? 1 : 0,
    );
    return `{${entries.map(([key, item]) => `${JSON.stringify(key)}:${canonical(item)}`).join(",")}}`;
  }
  return JSON.stringify(value);
}

function unexpected(
  step: number | undefined,
  expected: string,
  message: Record<string, unknown> | undefined,
  line: string,
): TestSupportError {
  const received = message === undefined ? JSON.stringify(line) : canonical(message);
  const where = step === undefined ? "the script expects" : `step ${step} expects`;
  return new TestSupportError(
    "unexpected_message",
    `unexpected_message: ${where} ${expected}, and the Client sent ${received}`,
  );
}

function unusedStep(step: number): TestSupportError {
  return new TestSupportError("unused_step", `unused_step: step ${step} was left unused`);
}

function nothingToAnswer(step: number): TestSupportError {
  return new TestSupportError(
    "nothing_to_answer",
    `nothing_to_answer: step ${step} answers a request, and none came before it`,
  );
}
