// Seam B of the Scripted engine's players (ADR 0069): the in-process player
// plays the shared corpus to a tiny Client written here, over the in-memory
// transport an SDK takes, as `goodfetch-scripted-engine` plays it over stdio.
import { describe, expect, test } from "vitest";
import { type InMemoryTransport, playScript } from "./play-script.ts";

const encoder = new TextEncoder();

/** The hello every self-test script expects from a Collie Client. */
const collieHello = {
  protocol_version: "0.1",
  surface: "sdk_typescript",
  form: "typescript",
  needs: "collie",
  product_version: "0.1.0",
};

/** A Client that writes and reads newline-delimited JSON-RPC, and nothing more. */
class TinyClient {
  readonly #writer: WritableStreamDefaultWriter<Uint8Array>;
  readonly #lines: AsyncIterator<string>;
  #nextId = 0;

  constructor(transport: InMemoryTransport) {
    this.#writer = transport.writable.getWriter();
    this.#lines = lines(transport.readable);
  }

  /** Writes one line of text. */
  async write(text: string): Promise<void> {
    await this.#writer.write(encoder.encode(text));
  }

  async send(message: object): Promise<void> {
    await this.write(`${JSON.stringify(message)}\n`);
  }

  /** Sends a request with the next ID, and returns the ID. */
  async request(method: string, params: object): Promise<number> {
    const id = this.#nextId++;
    await this.send({ jsonrpc: "2.0", id, method, params });
    return id;
  }

  /** The engine's next line, or `undefined` when it closed its end. */
  async nextLine(): Promise<string | undefined> {
    const line = await this.#lines.next();
    return line.done === true ? undefined : line.value;
  }

  /** The engine's next message, or `undefined` when it closed its end. */
  async next(): Promise<unknown> {
    const line = await this.nextLine();
    return line === undefined ? undefined : JSON.parse(line);
  }

  /** Closes the Client's end. */
  async close(): Promise<void> {
    await this.#writer.close();
  }
}

/** The lines of `readable`, each without its newline. */
async function* lines(readable: ReadableStream<Uint8Array>): AsyncGenerator<string> {
  const decoder = new TextDecoder();
  let buffered = "";
  for await (const chunk of readable) {
    buffered += decoder.decode(chunk, { stream: true });
    let end = buffered.indexOf("\n");
    while (end !== -1) {
      yield buffered.slice(0, end);
      buffered = buffered.slice(end + 1);
      end = buffered.indexOf("\n");
    }
  }
  if (buffered !== "") yield buffered;
}

describe("playScript", () => {
  test.for([
    { script: "self-test/hello-as-collie.json", product: "collie", productVersion: "0.1.0" },
    { script: "self-test/hello-as-foxhound.json", product: "foxhound", productVersion: "0.2.0" },
  ])(
    "a script replying as $product is read as that Product's engine",
    async ({ script, product, productVersion }) => {
      const played = playScript(script);
      const client = new TinyClient(played.transport);

      const id = await client.request("hello", collieHello);
      const reply = await client.next();
      await client.close();

      expect(reply).toEqual({
        jsonrpc: "2.0",
        id,
        result: {
          protocol_version: "0.1",
          product,
          product_version: productVersion,
          engine_instance: "0123456789abcdef",
        },
      });
      expect(played.done()).toEqual({ kind: "finished" });
    },
  );

  test("a self-test script plays a call to its end", async () => {
    const played = playScript("self-test/call-answered.json");
    const client = new TinyClient(played.transport);

    await client.request("hello", collieHello);
    await client.next();
    const id = await client.request("engine.info", {});
    const reply = await client.next();
    await client.close();

    expect(reply).toEqual({
      jsonrpc: "2.0",
      id,
      result: {
        product: "collie",
        product_version: "0.1.0",
        protocol_version: "0.1",
        product_major: 0,
      },
    });
    expect(played.done()).toEqual({ kind: "finished" });
  });

  test("a script refusing the Client's Product with not_in_product ends the exchange there", async () => {
    const played = playScript("self-test/hello-refused-not-in-product.json");
    const client = new TinyClient(played.transport);

    await client.request("hello", { ...collieHello, needs: "foxhound" });
    const reply = await client.next();
    const after = await client.next();

    expect(reply).toMatchObject({ error: { data: { code: "not_in_product" } } });
    expect(after).toBeUndefined();
    expect(played.done()).toEqual({ kind: "closed" });
  });

  test("a script calls the Client's handler and sends an event during a call", async () => {
    const played = playScript("self-test/handler-call-and-event.json");
    const client = new TinyClient(played.transport);

    await client.request("hello", collieHello);
    await client.next();
    const call = await client.request("sample.echo", { text: "hello" });
    const handlerCall = await client.next();
    await client.send({ jsonrpc: "2.0", id: 1, result: { replaced: false } });
    const event = await client.next();
    const reply = await client.next();
    await client.close();

    expect([handlerCall, event, reply]).toEqual([
      {
        jsonrpc: "2.0",
        id: 1,
        method: "sample_store.put",
        params: { key: "greeting", value: "hello" },
      },
      { jsonrpc: "2.0", method: "sample.echoed", params: { text: "hello" } },
      { jsonrpc: "2.0", id: call, result: { text: "hello" } },
    ]);
    expect(played.done()).toEqual({ kind: "finished" });
  });

  test("a handler's wrong answer fails the play at once", async () => {
    const played = playScript("self-test/handler-call-and-event.json");
    const client = new TinyClient(played.transport);
    await client.request("hello", collieHello);
    await client.next();
    await client.request("sample.echo", { text: "hello" });
    await client.next();

    const answered = client.send({ jsonrpc: "2.0", id: 1, result: { replaced: true } });

    await expect(answered).rejects.toMatchObject({ code: "unexpected_message" });
  });

  test("a script sends events once the Client grants credits, then closes", async () => {
    const played = playScript("self-test/events-on-credit.json");
    const client = new TinyClient(played.transport);
    await client.request("hello", collieHello);
    await client.next();
    const subscription = await client.request("events.subscribe", {});

    await client.send({
      jsonrpc: "2.0",
      method: "$/credit",
      params: { id: subscription, credits: 1 },
    });
    const batch = await client.next();
    const after = await client.next();

    expect([batch, after]).toEqual([
      {
        jsonrpc: "2.0",
        method: "$/events",
        params: {
          id: 1,
          events: [{ position: 1, event: "sample.echoed", data: { text: "hello" } }],
        },
      },
      undefined,
    ]);
    expect(played.done()).toEqual({ kind: "closed" });
  });

  test("a reply after an expected cancel answers the call it cancels", async () => {
    const played = playScript("self-test/call-cancelled.json");
    const client = new TinyClient(played.transport);
    await client.request("hello", collieHello);
    await client.next();
    const subscription = await client.request("events.subscribe", {});

    await client.send({ jsonrpc: "2.0", method: "$/cancel", params: { id: subscription } });
    const reply = await client.next();
    await client.close();

    expect(reply).toMatchObject({ id: subscription, error: { data: { code: "call_cancelled" } } });
    expect(played.done()).toEqual({ kind: "finished" });
  });

  test("a stalled script answers nothing until the Client gives up", async () => {
    const played = playScript("self-test/stall.json");
    const client = new TinyClient(played.transport);
    await client.request("hello", collieHello);
    await client.next();
    await client.request("sample.echo", { text: "hello" });

    await client.close();

    expect(await client.next()).toBeUndefined();
    expect(played.done()).toEqual({ kind: "finished" });
  });

  test("a message no step expects fails the play at once", async () => {
    const played = playScript("self-test/call-answered.json");
    const client = new TinyClient(played.transport);

    const sent = client.request("hello", { ...collieHello, needs: "foxhound" });

    await expect(sent).rejects.toMatchObject({ code: "unexpected_message" });
    await expect(client.next()).rejects.toMatchObject({ code: "unexpected_message" });
    expect(() => played.done()).toThrow(expect.objectContaining({ code: "unexpected_message" }));
  });

  test("a message without the jsonrpc member fails the play at once", async () => {
    const played = playScript("self-test/call-answered.json");
    const client = new TinyClient(played.transport);

    const sent = client.send({ id: 0, method: "hello", params: collieHello });

    await expect(sent).rejects.toMatchObject({ code: "unexpected_message" });
  });

  test("an answer with both a result and an error fails the play at once", async () => {
    const played = playScript("self-test/handler-call-and-event.json");
    const client = new TinyClient(played.transport);
    await client.request("hello", collieHello);
    await client.next();
    await client.request("sample.echo", { text: "hello" });
    await client.next();

    const answered = client.send({
      jsonrpc: "2.0",
      id: 1,
      result: { replaced: false },
      error: { code: -32000, message: "both" },
    });

    await expect(answered).rejects.toMatchObject({ code: "unexpected_message" });
  });

  test("an await_reply that follows no request fails with nothing_to_answer", () => {
    const script = new URL("fixtures/engine-scripts/await-reply-first.json", import.meta.url);

    const played = playScript(script);

    expect(() => played.done()).toThrow(expect.objectContaining({ code: "nothing_to_answer" }));
  });

  test("a JSON line that is not an object is recorded as a message", async () => {
    const played = playScript("self-test/call-answered.json");
    const client = new TinyClient(played.transport);

    const written = client.write("[1]\n");

    await expect(written).rejects.toMatchObject({ code: "unexpected_message" });
    expect(played.messages).toEqual([{ from: "client", message: [1] }]);
  });

  test("a line that is not JSON fails the play at once", async () => {
    const played = playScript("self-test/call-answered.json");
    const client = new TinyClient(played.transport);

    const written = client.write("hello\n");

    await expect(written).rejects.toMatchObject({ code: "unexpected_message" });
  });

  test("a message after the last step fails the play", async () => {
    const played = playScript("self-test/hello-as-collie.json");
    const client = new TinyClient(played.transport);
    await client.request("hello", collieHello);
    await client.next();

    const sent = client.request("engine.info", {});

    await expect(sent).rejects.toMatchObject({ code: "unexpected_message" });
  });

  test("a step left unused fails the play when the Client closes its end", async () => {
    const played = playScript("self-test/call-answered.json");
    const client = new TinyClient(played.transport);
    await client.request("hello", collieHello);
    await client.next();

    const closed = client.close();

    await expect(closed).rejects.toMatchObject({ code: "unused_step" });
    expect(() => played.done()).toThrow(expect.objectContaining({ code: "unused_step" }));
  });

  test("a step left unused fails done", async () => {
    const played = playScript("self-test/call-answered.json");
    const client = new TinyClient(played.transport);
    await client.request("hello", collieHello);
    await client.next();

    const done = (): unknown => played.done();

    expect(done).toThrow(expect.objectContaining({ code: "unused_step" }));
  });

  test("the crash mid-call script leaves the Client no answer and ends with its exit code", async () => {
    const played = playScript("attached-engine/crash-mid-call.json");
    const client = new TinyClient(played.transport);
    await client.request("hello", collieHello);
    await client.next();

    await client.request("sample.echo", { text: "hello" });

    expect(await client.next()).toBeUndefined();
    expect(played.done()).toEqual({ kind: "exit", code: 101 });
  });

  test("the garbage on stdout script gives the Client a line that is not JSON-RPC", async () => {
    const played = playScript("attached-engine/garbage-on-stdout.json");
    const client = new TinyClient(played.transport);
    await client.request("hello", collieHello);
    await client.next();

    const line = await client.nextLine();
    await client.close();

    expect(line).toBe("panicked at engine/src/lib.rs:1:1");
    expect(played.done()).toEqual({ kind: "finished" });
  });

  test("the play records every message it read and wrote", async () => {
    const played = playScript("self-test/hello-as-collie.json");
    const client = new TinyClient(played.transport);

    await client.request("hello", collieHello);
    await client.next();
    await client.close();

    expect(played.messages).toEqual([
      {
        from: "client",
        message: { jsonrpc: "2.0", id: 0, method: "hello", params: collieHello },
      },
      {
        from: "engine",
        message: {
          jsonrpc: "2.0",
          id: 0,
          result: {
            protocol_version: "0.1",
            product: "collie",
            product_version: "0.1.0",
            engine_instance: "0123456789abcdef",
          },
        },
      },
    ]);
  });

  test("a script that fails script.schema.json is refused", () => {
    const script = new URL("fixtures/engine-scripts/unknown-step.json", import.meta.url);

    const played = (): unknown => playScript(script);

    expect(played).toThrow(expect.objectContaining({ code: "fixture_invalid" }));
  });
});
