// A followed read (ADR 0102), such as `run.progress`, as a sequence's
// stream: its first read is sent when the sequence is made and answers at
// once; each next one is sent once the program reads the value before it,
// with that value's `position` as `after`, and the engine answers it with
// the latest value once a newer one comes, so values the program did not
// read in time coalesce. The value whose `last` is true ends it. A
// sequence's credit is one read: the program's own pace sets when the next
// is sent.
import type { Stream } from "./client-connection.ts";
import type { ChannelOpenOptions } from "./rpc.ts";

/** How a followed read is read. */
export interface FollowedReads<Value> {
  /** Sends one read: after the value at `after`, or at once without it. */
  readonly read: (after: number | undefined, signal: AbortSignal | undefined) => Promise<Value>;
  /** The value's position, the next read's `after`. */
  readonly position: (value: Value) => number;
  /** Whether the value is the last, which ends the reads. */
  readonly last: (value: Value) => boolean;
}

/** The method a followed read hands each value under, to a sequence's `itemOf`. */
export const FOLLOWED_VALUE = "$/followed";

/**
 * Opens `reads` as a sequence's stream: each value it reads reaches
 * `options.notified` as `FOLLOWED_VALUE`, and `take()` hands it, in order.
 */
export function followed<Value>(
  reads: FollowedReads<Value>,
  options: ChannelOpenOptions,
): { readonly stream: Stream; readonly take: () => Value | undefined } {
  const values: Value[] = [];
  let credits = 0;
  let granted: (() => void) | undefined;
  const signal = options.signal;
  signal?.addEventListener("abort", () => granted?.(), { once: true });
  const result = (async (): Promise<unknown> => {
    let after: number | undefined;
    for (;;) {
      if (credits === 0) {
        signal?.throwIfAborted();
        // Woken by a grant, which leaves credits, or by the abort.
        await new Promise<void>((resolve) => {
          granted = resolve;
        });
        granted = undefined;
      }
      signal?.throwIfAborted();
      credits -= 1;
      const value = await reads.read(after, signal);
      values.push(value);
      options.notified?.(FOLLOWED_VALUE, undefined);
      if (reads.last(value)) return {};
      after = reads.position(value);
    }
  })();
  return {
    stream: {
      result,
      grant: (more) => {
        credits += more;
        if (credits > 0) granted?.();
      },
    },
    take: () => values.shift(),
  };
}
