// A stream made resumable: what the Channel sends it again with on a new
// link, read at the drop from the items its notifications carried and the
// credits granted, over a stand-in for the Connection's `stream`.
import { describe, expect, test } from "vitest";
import type { Stream } from "./client-connection.ts";
import { resumable } from "./resumed-stream.ts";
import type { ChannelOpenOptions } from "./rpc.ts";

/** Opens a resumable stream over a stand-in, which keeps the options it was opened with. */
function opened(heard: unknown[]): { stream: Stream; options: ChannelOpenOptions } {
  let options: ChannelOpenOptions | undefined;
  const stream = resumable(
    (given) => {
      options = given;
      return { result: new Promise(() => undefined), grant: () => undefined };
    },
    { notified: (method, params) => heard.push([method, params]) },
    {
      read: (method, params) =>
        method === "$/item" && typeof params === "number" ? { count: 2, last: params } : undefined,
      params: (last) => (last === undefined ? { from: "start" } : { after: last }),
    },
  );
  if (options === undefined) {
    throw new TypeError("invariant: resumable opens its stream at once");
  }
  return { stream, options };
}

describe("a resumable stream", () => {
  test(
    "is sent again after the last item read, with the credits granted less the items received",
    { tags: ["story:11-6"] },
    () => {
      const heard: unknown[] = [];
      const { stream, options } = opened(heard);
      stream.grant(16);
      stream.grant(4);

      options.notified?.("$/item", 7);
      options.notified?.("$/other", 8);
      options.notified?.("$/item", 9);

      expect({ resumed: options.resume?.(), heard }).toEqual({
        resumed: { params: { after: 9 }, credits: 16 },
        heard: [
          ["$/item", 7],
          ["$/other", 8],
          ["$/item", 9],
        ],
      });
    },
  );

  test("is sent again from its start, with every credit granted, when no item was read", () => {
    const { stream, options } = opened([]);
    stream.grant(16);

    expect(options.resume?.()).toEqual({ params: { from: "start" }, credits: 16 });
  });
});
