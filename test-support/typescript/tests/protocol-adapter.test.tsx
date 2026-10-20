// Seam B (spec #326): a Console screen test runs in Vitest Browser Mode,
// where no file system is, so the in-memory adapter of the protocol reads
// the committed bundle there too, and answers a call there (ADR 0068).
import { describe, expect, test } from "vitest";
import { createProtocolAdapter } from "@goodfetch/test-support/protocol-adapter";

describe("createProtocolAdapter in a browser", () => {
  test("answers a call with the value the test set", async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("sample.echo", { text: "hello hello" });
    const received: string[] = [];
    const connection = adapter.transport.open({
      message: (text) => received.push(text),
      close: () => undefined,
    });

    connection.send(
      JSON.stringify({ jsonrpc: "2.0", id: 1, method: "sample.echo", params: { text: "hello" } }),
    );

    await expect
      .poll(() => received)
      .toEqual(['{"jsonrpc":"2.0","id":1,"result":{"text":"hello hello"}}']);
  });
});
