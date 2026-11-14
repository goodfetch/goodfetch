// Collie's raw JSON-RPC row of the Collie scenario (issue 2406): the
// instrument ADR 0019's target is read on. It speaks the protocol itself to
// the engine `engine.ts` starts, one message per line each way, with no SDK
// between: `hello`, then each operation the plan's steps map to, as the
// catalog names it, awaited one after another.
import {
  type CollieOperationTypes,
  type HelloParams,
  PRODUCT_VERSION,
  PROTOCOL_VERSION,
} from "@goodfetch/collie-client/generated";

import type { Engine } from "./engine.ts";
import { type Driver, type DriverPage, type DriverSession, READ, readText } from "./plan.ts";

/** A call's answer: its result, or the error the engine answered. */
type Answer =
  | { readonly ok: true; readonly result: unknown }
  | { readonly ok: false; readonly error: unknown };

/** A JSON-RPC client over `engine`'s stdio, one call per id. */
class Client {
  readonly #writer: WritableStreamDefaultWriter<Uint8Array>;
  readonly #waiting = new Map<number, (answer: Answer) => void>();
  readonly #reading: Promise<void>;
  readonly #encoder = new TextEncoder();
  #next = 1;
  #ended: Error | undefined;

  constructor(engine: Engine) {
    this.#writer = engine.transport.writable.getWriter();
    this.#reading = this.#read(engine.transport.readable);
  }

  /** Calls `method` with `params` and returns its result, or throws the engine's error. */
  async call<Name extends keyof CollieOperationTypes>(
    method: Name,
    params: CollieOperationTypes[Name]["params"],
  ): Promise<CollieOperationTypes[Name]["result"]>;
  async call(method: "hello", params: HelloParams): Promise<unknown>;
  async call(method: string, params: unknown): Promise<unknown> {
    if (this.#ended !== undefined) throw this.#ended;
    const id = this.#next;
    this.#next += 1;
    const answered = new Promise<Answer>((resolve) => this.#waiting.set(id, resolve));
    await this.#send({ id, method, params });
    const answer = await answered;
    if (answer.ok) return answer.result;
    throw new Error(`${method}: ${errorText(answer.error)}`);
  }

  /** Ends the engine's input, which ends the engine, and waits for its output to end. */
  async close(): Promise<void> {
    await this.#writer.close();
    await this.#reading;
  }

  async #send(message: Readonly<Record<string, unknown>>): Promise<void> {
    await this.#writer.write(
      this.#encoder.encode(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`),
    );
  }

  async #read(readable: ReadableStream<Uint8Array>): Promise<void> {
    const decoder = new TextDecoder();
    let partial = "";
    try {
      for await (const chunk of readable) {
        const lines = (partial + decoder.decode(chunk, { stream: true })).split("\n");
        partial = lines.pop() ?? "";
        for (const line of lines) await this.#heard(line);
      }
      this.#end(new Error("the engine ended its output"));
    } catch (error) {
      this.#end(error instanceof Error ? error : new Error(String(error)));
    }
  }

  /** One line the engine sent: an answer, a request it is refused, or a notification. */
  async #heard(line: string): Promise<void> {
    const message: unknown = JSON.parse(line);
    if (typeof message !== "object" || message === null) return;
    const fields: Record<string, unknown> = { ...message };
    const id = fields["id"];
    if (typeof fields["method"] === "string") {
      // No request reaches a Collie Connection: answer it as the SDK does.
      if (id !== undefined && id !== null) {
        await this.#send({
          id,
          error: { code: -32601, message: `this Client serves no \`${fields["method"]}\`` },
        });
      }
      return;
    }
    if (typeof id !== "number") return;
    const resolve = this.#waiting.get(id);
    if (resolve === undefined) return;
    this.#waiting.delete(id);
    resolve(
      "error" in fields
        ? { ok: false, error: fields["error"] }
        : { ok: true, result: fields["result"] },
    );
  }

  #end(error: Error): void {
    this.#ended = error;
    for (const resolve of this.#waiting.values()) resolve({ ok: false, error: error.message });
    this.#waiting.clear();
  }
}

/** The first line of the engine's error's message. */
function errorText(error: unknown): string {
  if (typeof error === "object" && error !== null) {
    const fields: Record<string, unknown> = { ...error };
    if (typeof fields["message"] === "string") return fields["message"].split("\n")[0] ?? "";
  }
  return String(error);
}

/** Collie over raw JSON-RPC to `engine`, its Sessions attached to `cdp`. */
export async function raw(engine: Engine, cdp: string): Promise<Driver> {
  const client = new Client(engine);
  await client.call("hello", {
    protocol_version: PROTOCOL_VERSION,
    surface: "sdk_typescript",
    needs: "collie",
    product_version: PRODUCT_VERSION,
  });
  const { run_id } = await client.call("run.start", {});
  const page = (page_id: string): DriverPage => ({
    async goto(url) {
      const navigated = await client.call("page.goto", { page_id, url });
      return navigated.response?.status ?? null;
    },
    async step(step) {
      if (step.action === "read") {
        const evaluated = await client.call("page.evaluate", { page_id, function: READ });
        return readText(evaluated.value);
      }
      const locator = { page_id, selector: step.selector };
      if (step.action === "click") await client.call("locator.click", { locator });
      else if (step.action === "fill") {
        await client.call("locator.fill", { locator, value: step.value });
      } else await client.call("locator.press", { locator, key: step.key });
      return null;
    },
    async close() {
      await client.call("page.close", { page_id });
    },
  });
  const session = (session_id: string): DriverSession => ({
    newPage: async () => page((await client.call("session.new_page", { session_id })).page_id),
    async close() {
      await client.call("session.close", { session_id });
    },
  });
  return {
    async newSession() {
      const opened = await client.call("run.new_session", {
        run_id,
        source: { kind: "cdp_url", url: cdp },
      });
      return session(opened.session_id);
    },
    async close() {
      await client.call("run.finish", { run_id });
      await client.close();
      await engine.exited;
    },
    sent: () => engine.sent(),
  };
}
