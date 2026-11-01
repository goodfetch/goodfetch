// Console-like tests whose Client the in-memory adapter of the protocol must
// catch: an unanswered call, an answer or a request breaking the bundle, and
// a body never served each fail the test, even though its own assertion,
// that the Client got nothing, holds; a call answered as the bundle allows
// passes.
import { expect, test } from "vitest";
import {
  createProtocolAdapter,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";

/** Sends one message on a new Connection, and resolves with what the engine sent back first, if anything, by the next task. */
async function send(adapter: ProtocolAdapter, message: Record<string, unknown>): Promise<string[]> {
  const received: string[] = [];
  const connection = adapter.transport.open({
    message: (text) => received.push(text),
    close: () => undefined,
  });
  connection.send(JSON.stringify({ jsonrpc: "2.0", ...message }));
  await new Promise((resolve) => setTimeout(resolve, 0));
  return received;
}

test("an unanswered call", async () => {
  const adapter = createProtocolAdapter();

  const received = await send(adapter, { id: 1, method: "sample.echo", params: { text: "hello" } });

  expect(received).toEqual([]);
});

test("an answer breaking its schema", async () => {
  const adapter = createProtocolAdapter();
  adapter.answer("run.start", () => ({
    run_id: "Not A Run ID",
    started_as: "new",
    lifecycle: "running",
    needs: "collie",
  }));

  const received = await send(adapter, { id: 1, method: "run.start", params: {} });

  expect(received).toEqual([]);
});

test("a request breaking its schema", async () => {
  const adapter = createProtocolAdapter();
  adapter.answer("sample.echo", { text: "hello hello" });

  const received = await send(adapter, { id: 1, method: "sample.echo", params: { text: 7 } });

  expect(received).toEqual([]);
});

test("a body never served", async () => {
  const adapter = createProtocolAdapter();

  const fetched = adapter.bodyFetcher("page-body-1");

  await expect(fetched).rejects.toMatchObject({ code: "body_unserved" });
});

test("a call answered as the bundle allows", async () => {
  const adapter = createProtocolAdapter();
  adapter.answer("sample.echo", { text: "hello hello" });

  const received = await send(adapter, { id: 1, method: "sample.echo", params: { text: "hello" } });

  expect(received).toEqual(['{"jsonrpc":"2.0","id":1,"result":{"text":"hello hello"}}']);
});

/** What the first test's Client heard after its test ended. */
const heardLate: string[] = [];

// oxlint-disable-next-line vitest/expect-expect -- it only sends a call; the test after it asserts what its Client heard
test("a test ending before its answer comes", async () => {
  const adapter = createProtocolAdapter();
  adapter.answer("sample.echo", async () => {
    await new Promise((resolve) => setTimeout(resolve, 50));
    return { text: "late" };
  });
  const connection = adapter.transport.open({
    message: (text) => heardLate.push(text),
    close: () => heardLate.push("closed"),
  });

  connection.send(
    JSON.stringify({ jsonrpc: "2.0", id: 1, method: "sample.echo", params: { text: "hi" } }),
  );
});

test("the test after it", async () => {
  await new Promise((resolve) => setTimeout(resolve, 200));

  expect(heardLate).toEqual([]);
});
