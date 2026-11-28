// A followed read (ADR 0102), such as `run.progress`, as a sequence's
// stream: its first read is sent when the sequence is made and answers at
// once; each next one is sent once the program reads the value before it,
// with that value's `position` as `after`, and the engine answers it with
// the latest value once a newer one comes, so values the program did not
// read in time coalesce. The value whose `last` is true ends it. A
// sequence's credit is one read: the program's own pace sets when the next
// is sent.
//
// A paged read, such as `run.rejected_records`, is read the same way, one
// page a read, each page handing its items in order; its credits count
// items, so a next page is asked for only while the program has credit left,
// though one page may hand more items than the credit left.
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

/** How a paged read is read: a followed read whose values are pages of items. */
export interface PagedReads<Page, Item> extends FollowedReads<Page> {
  /** The page's items, handed in order. */
  readonly items: (page: Page) => readonly Item[];
}

/** The method a followed or paged read hands each item under, to a sequence's `itemOf`. */
export const FOLLOWED_VALUE = "$/followed";

/** A followed or paged read opened as a sequence's stream. */
export interface Followed<Item> {
  readonly stream: Stream;
  /** Hands the next item read, in order. */
  readonly take: () => Item | undefined;
}

/**
 * Opens `reads` as a sequence's stream: each value it reads reaches
 * `options.notified` as `FOLLOWED_VALUE`, and `take()` hands it, in order.
 */
export function followed<Value>(
  reads: FollowedReads<Value>,
  options: ChannelOpenOptions,
): Followed<Value> {
  return paged({ ...reads, items: (value) => [value] }, options);
}

/**
 * Opens `reads` as a sequence's stream: each item of each page it reads
 * reaches `options.notified` as `FOLLOWED_VALUE`, and `take()` hands it, in
 * order. A page is read only while the sequence has credit left.
 */
export function paged<Page, Item>(
  reads: PagedReads<Page, Item>,
  options: ChannelOpenOptions,
): Followed<Item> {
  const items: Item[] = [];
  let credits = 0;
  let granted: (() => void) | undefined;
  const signal = options.signal;
  signal?.addEventListener("abort", () => granted?.(), { once: true });
  const result = (async (): Promise<unknown> => {
    let after: number | undefined;
    for (;;) {
      if (credits <= 0) {
        signal?.throwIfAborted();
        // Woken by a grant, which leaves credits, or by the abort.
        await new Promise<void>((resolve) => {
          granted = resolve;
        });
        granted = undefined;
      }
      signal?.throwIfAborted();
      const page = await reads.read(after, signal);
      const read = reads.items(page);
      credits -= read.length;
      for (const item of read) {
        items.push(item);
        options.notified?.(FOLLOWED_VALUE, undefined);
      }
      if (reads.last(page)) return {};
      after = reads.position(page);
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
    take: () => items.shift(),
  };
}
