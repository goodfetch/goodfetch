// A play's protocol counts, for an SDK test's snapshot of what a program
// cost on the wire: what the Client sent and what the engine sent, by kind.
// Bytes join with the SDKs' protocol-count seam (issue 1706).
import type { PlayedMessage } from "./play-script.ts";

/** What each side of a play sent, by kind. */
export interface ProtocolCounts {
  /** The method of each request and notification the Client sent, in order. */
  readonly requests: string[];
  /** How many answers the engine sent. */
  readonly answers: number;
  /** How many notifications the engine sent, such as events and streamed Records. */
  readonly events: number;
  /** The method of each request the engine sent to a handler of the Client's, in order. */
  readonly served: string[];
  /** How many answers the Client sent to the engine's requests. */
  readonly replies: number;
}

/** The play's protocol counts. */
export function protocolCounts(messages: readonly PlayedMessage[]): ProtocolCounts {
  const counts = {
    requests: [] as string[],
    answers: 0,
    events: 0,
    served: [] as string[],
    replies: 0,
  };
  for (const played of messages) {
    if (!("message" in played) || !isObject(played.message)) continue;
    const { message } = played;
    const method = typeof message["method"] === "string" ? message["method"] : undefined;
    if (played.from === "client") {
      if (method === undefined) counts.replies += 1;
      else counts.requests.push(method);
    } else if (method === undefined) {
      counts.answers += 1;
    } else if ("id" in message) {
      counts.served.push(method);
    } else {
      counts.events += 1;
    }
  }
  return counts;
}

function isObject(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null;
}
