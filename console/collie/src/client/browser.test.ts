// The Console's transport with its fallback (spec #592; ADR 0031): behind a
// proxy that refuses WebSockets, the next Connection goes over Server-Sent
// Events plus `POST`, and the one that carried an answer stays in use.
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { withFallback } from "./browser.ts";
import type { ConnectionListener, Transport } from "./client.ts";

/** A transport whose Connections the test ends, each open recorded under `name`. */
class Scripted implements Transport {
  readonly listeners: ConnectionListener[] = [];
  readonly name: string;
  readonly opened: string[];

  constructor(name: string, opened: string[]) {
    this.name = name;
    this.opened = opened;
  }

  open(listener: ConnectionListener) {
    this.opened.push(this.name);
    this.listeners.push(listener);
    return { send: () => undefined, close: () => listener.close() };
  }

  /** The last Connection answers a message. */
  answers(): void {
    this.listeners.at(-1)?.message("{}");
  }

  /** The last Connection ends. */
  ends(): void {
    this.listeners.at(-1)?.close();
  }
}

/** A WebSocket transport and an event-stream one, each open recorded in `opened`. */
function transports(): { webSocket: Scripted; eventStream: Scripted; opened: string[] } {
  const opened: string[] = [];
  return {
    webSocket: new Scripted("webSocket", opened),
    eventStream: new Scripted("eventStream", opened),
    opened,
  };
}

const noop: ConnectionListener = { message: () => undefined, close: () => undefined };

describe("the Console's transport with its fallback", () => {
  test(
    "opens over the event stream once a WebSocket ended unanswered, and keeps the one that answered",
    { tags: ["story:12-124"] },
    () => {
      const { webSocket, eventStream, opened } = transports();
      const transport = withFallback(webSocket, eventStream);

      transport.open(noop);
      webSocket.ends();
      transport.open(noop);
      eventStream.answers();
      eventStream.ends();
      transport.open(noop);

      expect(opened).toEqual(["webSocket", "eventStream", "eventStream"]);
    },
  );

  test("keeps the WebSocket after a drop of one that answered", { tags: ["story:12-124"] }, () => {
    const { webSocket, eventStream, opened } = transports();
    const transport = withFallback(webSocket, eventStream);

    transport.open(noop);
    webSocket.answers();
    webSocket.ends();
    transport.open(noop);

    expect(opened).toEqual(["webSocket", "webSocket"]);
  });

  test(
    "tries the WebSocket again once the event stream also ended unanswered",
    { tags: ["story:12-124"] },
    () => {
      const { webSocket, eventStream, opened } = transports();
      const transport = withFallback(webSocket, eventStream);

      transport.open(noop);
      webSocket.ends();
      transport.open(noop);
      eventStream.ends();
      transport.open(noop);

      expect(opened).toEqual(["webSocket", "eventStream", "webSocket"]);
    },
  );
});
