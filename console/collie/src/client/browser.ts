// What the Console's Client takes in production (ADR 0068, items 2 and 10):
// the WebSocket on the Console's own origin, which its sign-in cookie
// authenticates; a body fetcher by handle over plain HTTP `GET`; and the
// page's visibility, from `document.visibilityState`. Another gateway's
// build passes its own endpoint (ADR 0031).
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
