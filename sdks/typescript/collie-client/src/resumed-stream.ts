// A stream the Channel sends again on a new link once its link dropped
// (spec 606): a read whose engine starts it after the last item the Client
// read, such as `events.subscribe` after an event's position or
// `run.records` after a Record's number, granted the credits it still had.
import type { Stream } from "./client-connection.ts";
import type { ChannelOpenOptions } from "./rpc.ts";

/** The items one notification of a stream carries, each of which used one credit. */
export interface ReadItems<Position> {
  readonly count: number;
  /** Where the last of them stands, which the stream starts after when sent again. */
  readonly last: Position | undefined;
}

/** How a stream's notifications are read, and what it is sent again with. */
export interface ResumedAfter<Position> {
  /** The items `method`'s notification carries; `undefined` for a notification that carries none. */
  readonly read: (method: string, params: unknown) => ReadItems<Position> | undefined;
  /** The stream's params after `last`, or from its start when no item was read. */
  readonly params: (last: Position | undefined) => unknown;
}

/**
 * The stream `open` sends, given `opened`, that the Channel sends again on
 * a new link as `after` says, with the credits granted less the items
 * received.
 */
export function resumable<Position>(
  open: (options: ChannelOpenOptions) => Stream,
  opened: ChannelOpenOptions,
  after: ResumedAfter<Position>,
): Stream {
  let last: Position | undefined;
  let granted = 0;
  let received = 0;
  const stream = open({
    ...opened,
    notified: (method, params) => {
      const read = after.read(method, params);
      if (read !== undefined) {
        received += read.count;
        last = read.last ?? last;
      }
      opened.notified?.(method, params);
    },
    resume: () => ({ params: after.params(last), credits: granted - received }),
  });
  return {
    result: stream.result,
    grant: (credits) => {
      granted += credits;
      stream.grant(credits);
    },
  };
}
