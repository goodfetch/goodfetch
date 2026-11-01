// The engine's side of the framing, read from bytes split and merged as a
// pipe delivers them.
import { describe, expect, test } from "vitest";
import { readLines, type Line } from "./framing.ts";

/** A stream that delivers `chunks` as they are. */
function delivered(...chunks: readonly Uint8Array[]): ReadableStream<Uint8Array> {
  return new ReadableStream({
    start(controller) {
      for (const chunk of chunks) {
        controller.enqueue(chunk);
      }
      controller.close();
    },
  });
}

async function linesOf(readable: ReadableStream<Uint8Array>): Promise<Line[]> {
  const lines: Line[] = [];
  for await (const line of readLines(readable)) {
    lines.push(line);
  }
  return lines;
}

const encoder = new TextEncoder();

describe("readLines", () => {
  test("reads one message split over chunks, and two merged in one", async () => {
    const lines = await linesOf(
      delivered(
        encoder.encode('{"jsonrpc":"2.0","id":0,'),
        encoder.encode('"result":{}}\n{"jsonrpc":"2.0","method":"a"}\n{"jsonrpc":"2.0",'),
        encoder.encode('"method":"b"}\n'),
      ),
    );

    expect(lines).toEqual([
      { kind: "message", message: { kind: "result", id: 0, result: {} } },
      { kind: "message", message: { kind: "notification", method: "a", params: undefined } },
      { kind: "message", message: { kind: "notification", method: "b", params: undefined } },
    ]);
  });

  test("reads a character whose UTF-8 bytes are split across chunks", async () => {
    const bytes = encoder.encode('{"jsonrpc":"2.0","method":"é"}\n');
    const split = bytes.indexOf(0xc3) + 1;

    const lines = await linesOf(delivered(bytes.slice(0, split), bytes.slice(split)));

    expect(lines).toEqual([
      { kind: "message", message: { kind: "notification", method: "é", params: undefined } },
    ]);
  });

  test("reports a line that is not UTF-8 as invalid, never as a message", async () => {
    const before = encoder.encode('{"jsonrpc":"2.0","method":"');
    const after = encoder.encode('"}\n');

    const lines = await linesOf(delivered(before, Uint8Array.of(0xff), after));

    expect(lines).toEqual([{ kind: "invalid", line: '{"jsonrpc":"2.0","method":"�"}' }]);
  });
});
