// What the Console's Client takes in production (ADR 0068, items 2 and 10):
// the WebSocket on the Console's own origin, which its sign-in cookie
// authenticates, falling back to Server-Sent Events plus `POST` with the
// sign-in's CSRF token behind a proxy that refuses WebSockets (ADR 0031); a
// body fetcher by handle over plain HTTP `GET`; and the page's visibility,
// from `document.visibilityState`. Another gateway's build passes its own
// endpoint (ADR 0031).
import type { BodyFetcher, PageVisibility, Transport } from "./client.ts";

/** Where the Console's WebSocket opens, on the origin it was served from. */
export function protocolUrl(origin: string): string {
  const url = new URL("/protocol", origin);
  url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
  return url.href;
}

/** Connections over a WebSocket at `url`, one JSON-RPC message per text message. */
export function webSocketTransport(url: string): Transport {
  return {
    open(listener) {
      const socket = new WebSocket(url);
      const waiting: string[] = [];
      socket.addEventListener("open", () => {
        for (const text of waiting.splice(0)) {
          socket.send(text);
        }
      });
      socket.addEventListener("message", (event: MessageEvent<unknown>) => {
        if (typeof event.data === "string") {
          listener.message(event.data);
        }
      });
      socket.addEventListener("close", () => listener.close());
      return {
        send(text) {
          if (socket.readyState === WebSocket.OPEN) {
            socket.send(text);
          } else if (socket.readyState === WebSocket.CONNECTING) {
            waiting.push(text);
          }
        },
        close() {
          socket.close();
        },
      };
    },
  };
}

/** Where the Console's event stream opens, on the origin it was served from. */
export function eventStreamUrl(origin: string): string {
  return new URL("/protocol/events", origin).href;
}

/**
 * Connections over Server-Sent Events plus `POST` (ADR 0031), for a network
 * whose proxy refuses WebSockets: `GET /protocol/events` streams each
 * message the engine sends as one event's data, after a first `stream`
 * event naming where the page posts; each `POST /protocol/messages/
 * <stream>` carries the messages waiting, one per line, in order, with the
 * sign-in's CSRF token, which `fetcher` sends. A refused `POST` or a failed
 * stream ends the Connection: the Client opens a new one, never the
 * `EventSource`'s own reconnection.
 */
export function eventStreamTransport(
  origin: string,
  csrfToken: string,
  fetcher: typeof fetch,
): Transport {
  return {
    open(listener) {
      const source = new EventSource(eventStreamUrl(origin));
      const waiting: string[] = [];
      let stream: string | undefined;
      let posting = false;
      let ended = false;
      const end = (): void => {
        if (!ended) {
          ended = true;
          source.close();
          listener.close();
        }
      };
      const post = async (): Promise<void> => {
        if (posting || ended || stream === undefined || waiting.length === 0) {
          return;
        }
        posting = true;
        const body = waiting.splice(0).join("\n");
        try {
          const answer = await fetcher(new URL(`/protocol/messages/${stream}`, origin), {
            method: "POST",
            body,
            credentials: "same-origin",
            headers: { "Content-Type": "text/plain;charset=UTF-8", "X-CSRF-Token": csrfToken },
          });
          if (!answer.ok) {
            end();
          }
        } catch {
          end();
        }
        posting = false;
        await post();
      };
      source.addEventListener("stream", (event: MessageEvent<unknown>) => {
        if (typeof event.data === "string") {
          stream = event.data;
          void post();
        }
      });
      source.addEventListener("message", (event: MessageEvent<unknown>) => {
        if (typeof event.data === "string" && !ended) {
          listener.message(event.data);
        }
      });
      source.addEventListener("error", end);
      return {
        send(text) {
          if (!ended) {
            waiting.push(text);
            void post();
          }
        },
        close: end,
      };
    },
  };
}

/**
 * Connections over `first`, the WebSocket, until one ends before the engine
 * sent anything, as behind a proxy that refuses WebSockets: the next opens
 * over `second`, the event stream, and so on, each failure turning to the
 * other; the transport of a Connection that carried a message stays in
 * use.
 */
export function withFallback(first: Transport, second: Transport): Transport {
  let current = first;
  return {
    open(listener) {
      const used = current;
      let heard = false;
      return used.open({
        message(text) {
          heard = true;
          listener.message(text);
        },
        close() {
          if (!heard && current === used) {
            current = used === first ? second : first;
          }
          listener.close();
        },
      });
    },
  };
}

/**
 * Reads a Run's file by its handle, `<run_id>/<body>`, from the Console's
 * origin with `fetcher`: `GET /bodies/<run_id>/<body>`, which the sign-in
 * cookie authorizes. The Console's origin answers it as an attachment it
 * never shows; a file is shown only from the content origin.
 */
export function httpBodyFetcher(origin: string, fetcher: typeof fetch): BodyFetcher {
  return async (handle) => {
    const path = handle.split("/").map(encodeURIComponent).join("/");
    const response = await fetcher(new URL(`/bodies/${path}`, origin), {
      credentials: "same-origin",
    });
    if (!response.ok) {
      throw new Error(`the Console's origin answered ${response.status} for the body ${handle}`);
    }
    return new Uint8Array(await response.arrayBuffer());
  };
}

/** The visibility of `document`'s page. */
export function documentVisibility(document: Document): PageVisibility {
  return {
    get visible() {
      return document.visibilityState === "visible";
    },
    watch(changed) {
      document.addEventListener("visibilitychange", changed);
      return () => document.removeEventListener("visibilitychange", changed);
    },
  };
}
