// One subscription to the engine's event stream over one Connection (ADR
// 0031; spec #216): `events.subscribe` stays in flight, resuming after the
// position its `after` names when it is the same engine instance's, and the
// engine sends its events in `$/events` batches as the credits the Client
// grants with `$/credit` allow. Across Connections, the Client keeps the
// last position it read and subscribes again after it.
import type { MessageId } from "./framing.ts";
import type { Channel } from "./rpc.ts";

/** The last event a Client read: its engine instance, from `hello`, and its position. */
export interface LastRead {
  readonly engine_instance: string;
  readonly position: number;
}

/** One event of a `$/events` batch, as the wire carries it. */
export interface StreamedEvent {
  readonly position: number;
  readonly event: string;
  readonly data: unknown;
  /** How many telemetry events of its scope it replaced since the last one sent. */
  readonly dropped?: number | undefined;
}

/** How to subscribe. */
export interface EventsSubscriptionOptions {
  /** Resumes after this event; from now when absent. */
  readonly after?: LastRead | undefined;
  /** Ends the subscription with `$/cancel`. */
  readonly signal?: AbortSignal | undefined;
  /** Hears each batch, in position order. */
  received(events: readonly StreamedEvent[]): void;
}

/** A subscription in flight on one Connection. */
export interface EventsSubscription {
  /** Its call's id, which its batches and credits name. */
  readonly id: MessageId;
  /** Settles once the subscription ends: cancelled, or its Connection gone. */
  readonly ended: Promise<unknown>;
  /** Lets the engine send `credits` more events. */
  grant(credits: number): void;
}

function isRecord(value: unknown): value is Readonly<Record<string, unknown>> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

function isStreamedEvent(value: unknown): value is StreamedEvent {
  return (
    isRecord(value) &&
    typeof value["position"] === "number" &&
    typeof value["event"] === "string" &&
    "data" in value
  );
}

/** The events of a `$/events` batch's params; none when they are no batch. */
export function batchOf(params: unknown): readonly StreamedEvent[] {
  const events = isRecord(params) ? params["events"] : undefined;
  return Array.isArray(events) ? events.filter(isStreamedEvent) : [];
}

/** Subscribes to the event stream on `channel`; no event comes before a grant. */
export function subscribeEvents(
  channel: Channel,
  options: EventsSubscriptionOptions,
): EventsSubscription {
  const params = options.after === undefined ? {} : { after: options.after };
  const { id, result } = channel.open("events.subscribe", params, {
    signal: options.signal,
    notified: (method, notification) => {
      if (method === "$/events") {
        const events = batchOf(notification);
        if (events.length > 0) {
          options.received(events);
        }
      }
    },
  });
  return {
    id,
    ended: result.then(
      (value) => value,
      (error: unknown) => error,
    ),
    grant: (credits) => channel.notify("$/credit", { id, credits }),
  };
}
