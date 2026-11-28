// SDK handlers conformance cases (ADRs 0033, 0104 and 0132; #1029):
// registering Extractor handlers on `connectOver`/`connect`, routing
// `extract.content`, preserving `GoodfetchError` codes while wrapping other
// exceptions in `extract_handler_failed` (with handler, type, message and
// stack), cancelling on `$/cancel`, and running under the incoming request's
// `_meta` Trace context so child spans nest under the engine's.
import {
  ExtractHandler,
  SettingInvalidError,
  ValueInvalidError,
  type ExtractContentParams,
  type ExtractContentResult,
} from "@goodfetch/collie";
import { connectOver } from "@goodfetch/collie/testing";
import { type Transport } from "@goodfetch/collie-client/runtime";
import { context, trace } from "@opentelemetry/api";
import { AsyncLocalStorageContextManager } from "@opentelemetry/context-async-hooks";
import { afterAll, beforeAll, describe, expect, test } from "vitest";

const encoder = new TextEncoder();
const decoder = new TextDecoder();

interface WireMessage {
  readonly id?: number | string;
  readonly method?: string;
  readonly params?: Record<string, unknown>;
  readonly result?: Record<string, unknown>;
  readonly error?: {
    readonly code: number;
    readonly message: string;
    readonly data?: Record<string, unknown>;
  };
}

function parseWireMessage(line: string): WireMessage {
  const value: unknown = JSON.parse(line);
  if (typeof value !== "object" || value === null) {
    throw new TypeError(`expected JSON object line, got ${typeof value}`);
  }
  return value;
}

/** An in-memory JSON-RPC transport driven by a test script acting as the engine. */
function scriptedTransport(): {
  readonly transport: Transport;
  readonly sendFromEngine: (message: Record<string, unknown>) => void;
  readonly nextFromClient: () => Promise<WireMessage>;
  readonly closeFromEngine: () => void;
} {
  let engineController: ReadableStreamDefaultController<Uint8Array> | undefined;
  const readable = new ReadableStream<Uint8Array>({
    start(controller) {
      engineController = controller;
    },
  });
  const received: WireMessage[] = [];
  const waiters: Array<(message: WireMessage) => void> = [];
  let buffer = "";
  const writable = new WritableStream<Uint8Array>({
    write(chunk) {
      buffer += decoder.decode(chunk, { stream: true });
      for (;;) {
        const newline = buffer.indexOf("\n");
        if (newline === -1) {
          break;
        }
        const line = buffer.slice(0, newline);
        buffer = buffer.slice(newline + 1);
        if (line.trim() === "") {
          continue;
        }
        const parsed = parseWireMessage(line);
        const waiter = waiters.shift();
        if (waiter !== undefined) {
          waiter(parsed);
        } else {
          received.push(parsed);
        }
      }
    },
  });
  return {
    transport: { readable, writable },
    sendFromEngine(message) {
      engineController?.enqueue(
        encoder.encode(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`),
      );
    },
    nextFromClient() {
      const queued = received.shift();
      if (queued !== undefined) {
        return Promise.resolve(queued);
      }
      return new Promise<WireMessage>((resolve) => {
        waiters.push(resolve);
      });
    },
    closeFromEngine() {
      engineController?.close();
    },
  };
}

function helloResult(): Record<string, unknown> {
  return {
    protocol_version: "0.1",
    product: "collie",
    product_version: "0.0.0",
    engine_instance: "scripted-1",
    timeouts: [],
  };
}

class CustomExtractor extends ExtractHandler {
  readonly #fn: (
    params: ExtractContentParams,
    options?: { readonly signal?: AbortSignal },
  ) => Promise<ExtractContentResult>;

  constructor(
    fn: (
      params: ExtractContentParams,
      options?: { readonly signal?: AbortSignal },
    ) => Promise<ExtractContentResult>,
  ) {
    super();
    this.#fn = fn;
  }

  override content(
    params: ExtractContentParams,
    options?: { readonly signal?: AbortSignal },
  ): Promise<ExtractContentResult> {
    return this.#fn(params, options);
  }
}

beforeAll(() => {
  context.setGlobalContextManager(new AsyncLocalStorageContextManager().enable());
});

afterAll(() => {
  context.disable();
});

describe("SDK handlers in TypeScript", () => {
  // Covers: story:7-72, story:7-73, story:7-114, story:7-76
  test("declares handlers in hello, serves extract.content, preserves GoodfetchError, wraps exceptions, cancels on $/cancel, and nests under incoming Trace context", async () => {
    const peer = scriptedTransport();
    const seenTrace: Array<{
      traceId: string | undefined;
      spanId: string | undefined;
      tracestate: string | undefined;
    }> = [];
    let aborted = false;
    let releaseBlocked: (() => void) | undefined;

    const extractor = new CustomExtractor(async (params, options) => {
      const span = trace.getSpanContext(context.active());
      seenTrace.push({
        traceId: span?.traceId,
        spanId: span?.spanId,
        tracestate: span?.traceState?.serialize(),
      });
      if (params.url.includes("goodfetch-error")) {
        throw new ValueInvalidError("bad html", { details: { value: "html" } });
      }
      if (params.url.includes("boom")) {
        throw new RangeError("extractor exploded");
      }
      if (params.url.includes("slow")) {
        await new Promise<void>((resolve) => {
          releaseBlocked = resolve;
          options?.signal?.addEventListener(
            "abort",
            () => {
              aborted = true;
              resolve();
            },
            { once: true },
          );
        });
        return { title: "Late", text: "late", markdown: "late", metadata: {} };
      }
      return {
        title: "Article",
        text: `text:${params.html}`,
        markdown: `# ${params.html}`,
        metadata: { source_kind: "sdk" },
      };
    });

    const connecting = connectOver(peer.transport, {
      extractors: { custom: extractor },
    });

    const hello = await peer.nextFromClient();
    expect(hello.method).toBe("hello");
    expect(hello.params?.["handlers"]).toEqual({ extract: ["custom"] });
    peer.sendFromEngine({ id: hello.id, result: helloResult() });
    const collie = await connecting;

    // 1. Happy path + incoming W3C Trace context (_meta).
    peer.sendFromEngine({
      id: 101,
      method: "extract.content",
      params: {
        url: "https://example.test/article",
        html: "<p>hello</p>",
        handler: "custom",
        _meta: {
          traceparent: "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01",
          tracestate: "vendor=ok",
        },
      },
    });
    const okReply = await peer.nextFromClient();
    expect(okReply.id).toBe(101);
    expect(okReply.result).toEqual({
      title: "Article",
      text: "text:<p>hello</p>",
      markdown: "# <p>hello</p>",
      metadata: { source_kind: "sdk" },
    });
    expect(seenTrace[0]).toEqual({
      traceId: "4bf92f3577b34da6a3ce929d0e0e4736",
      spanId: "00f067aa0ba902b7",
      tracestate: "vendor=ok",
    });

    // 2. GoodfetchError keeps its code.
    peer.sendFromEngine({
      id: 102,
      method: "extract.content",
      params: {
        url: "https://example.test/goodfetch-error",
        html: "<p>bad</p>",
        handler: "custom",
      },
    });
    const gfReply = await peer.nextFromClient();
    expect(gfReply.id).toBe(102);
    expect(gfReply.error?.data?.["code"]).toBe("value_invalid");

    // 3. Any other exception becomes extract_handler_failed with handler, type, message and stack.
    peer.sendFromEngine({
      id: 103,
      method: "extract.content",
      params: {
        url: "https://example.test/boom",
        html: "<p>boom</p>",
        handler: "custom",
      },
    });
    const boomReply = await peer.nextFromClient();
    expect(boomReply.id).toBe(103);
    expect(boomReply.error?.data?.["code"]).toBe("extract_handler_failed");
    expect(boomReply.error?.data?.["handler"]).toBe("custom");
    expect(boomReply.error?.data?.["extractor"]).toBe("custom");
    expect(boomReply.error?.data?.["exception_type"]).toBe("RangeError");
    expect(boomReply.error?.data?.["exception_message"]).toBe("extractor exploded");
    expect(String(boomReply.error?.data?.["exception_stack"])).toContain("RangeError");

    // 4. $/cancel aborts the async handler's signal and replies call_cancelled (late answer discarded).
    peer.sendFromEngine({
      id: 104,
      method: "extract.content",
      params: {
        url: "https://example.test/slow",
        html: "<p>slow</p>",
        handler: "custom",
      },
    });
    await new Promise((resolve) => setTimeout(resolve, 10));
    peer.sendFromEngine({ method: "$/cancel", params: { id: 104 } });
    const cancelReply = await peer.nextFromClient();
    expect(cancelReply.id).toBe(104);
    expect(cancelReply.error?.data?.["code"]).toBe("call_cancelled");
    expect(aborted).toBe(true);
    releaseBlocked?.();

    const closing = collie.close();
    peer.closeFromEngine();
    await closing;
  });

  // Covers: story:7-72, error:setting_invalid
  test("fails connect() with setting_invalid when the engine refuses ambiguous or clashing handlers", async () => {
    const peer = scriptedTransport();
    const dummy = new CustomExtractor(async () => ({
      title: "",
      text: "",
      markdown: "",
      metadata: {},
    }));

    const connecting = connectOver(peer.transport, {
      extractors: { first: dummy, second: dummy },
    });

    const hello = await peer.nextFromClient();
    expect(hello.params?.["handlers"]).toEqual({ extract: ["first", "second"] });
    peer.sendFromEngine({
      id: hello.id,
      error: {
        code: -32001,
        message: "`content.extractor` is invalid: two handlers registered",
        data: {
          code: "setting_invalid",
          retryable: false,
          key: "content.extractor",
          reason:
            "registered 2 handlers for `extract` (first, second) without setting `content.extractor`",
        },
      },
    });
    peer.closeFromEngine();

    await expect(connecting).rejects.toBeInstanceOf(SettingInvalidError);
  });
});
