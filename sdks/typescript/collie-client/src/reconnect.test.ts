// The backoff between attempts to reach a Daemon again after its link
// dropped: 250 ms, doubling, capped at 10 s, all within `timeout.connect`.
import { describe, expect, test } from "vitest";
import { reconnectDelay } from "./reconnect.ts";

describe("reconnectDelay", () => {
  test(
    "waits 250 ms before the first attempt and doubles each time up to 10 s",
    { tags: ["story:11-6"] },
    () => {
      const delays = Array.from({ length: 9 }, (_, attempt) => reconnectDelay(attempt, 0, null));

      expect(delays).toEqual([250, 500, 1_000, 2_000, 4_000, 8_000, 10_000, 10_000, 10_000]);
    },
  );

  test(
    "gives up once the next attempt would start past timeout.connect, counted from the drop",
    { tags: ["story:11-6"] },
    () => {
      expect({
        inside: reconnectDelay(3, 27_999, 30_000),
        atTheEnd: reconnectDelay(3, 28_000, 30_000),
        past: reconnectDelay(6, 25_750, 30_000),
      }).toEqual({ inside: 2_000, atTheEnd: undefined, past: undefined });
    },
  );
});
