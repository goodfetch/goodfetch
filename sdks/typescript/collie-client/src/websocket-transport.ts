// The protocol over WebSocket to an Exposed Daemon or Goodfetch Cloud
// (ADR 0049; spec #606's Transports): the subprotocol `goodfetch.v<major>`
// of the protocol version the SDK speaks, and the same JSON-RPC messages as
// over stdio, one per WebSocket message each way, so a Connection's
// newline-delimited transport runs over it unchanged. It uses the runtime's
// own `WebSocket`, with no Node built-in, so a browser's runtime can carry
// it too.
import { WEBSOCKET_SUBPROTOCOL } from "./generated/connection.ts";
import type { Transport } from "./rpc.ts";

/** What opening a WebSocket to a Daemon takes. */
export interface WebSocketOpenOptions {
  /**
   * The `Origin` its handshake sends, which a Client outside a browser
   * sets to the Daemon's own (ADRs 0049 and 0067); a browser sends its
   * page's and takes none.
   */
  readonly origin?: string | undefined;
  /** Cancels the opening: it rejects with the signal's reason. */
  readonly signal?: AbortSignal | undefined;
}

/**
 * Opens a WebSocket to `url` offering {@link WEBSOCKET_SUBPROTOCOL}, and
 * resolves to the Connection's transport over it once the Daemon took the
 * subprotocol, or to `undefined` when the handshake failed: nothing
 * answered, the Daemon refused it, or it took no subprotocol, which a
 * WebSocket does not tell apart.
 *
 * @throws the signal's reason once `signal` aborts.
 */
export async function openWebSocket(
  url: string,
  options: WebSocketOpenOptions = {},
): Promise<Transport | undefined> {
  const { origin, signal } = options;
  signal?.throwIfAborted();
  const protocols = [WEBSOCKET_SUBPROTOCOL];
  // A runtime outside a browser takes the handshake's headers with its
  // protocols (undici's `WebSocketInit`, in Node, Deno and Bun); a browser's
  // takes the protocols alone, and a program with the DOM's typing says so,
  // so the headers go through `Reflect.construct`.
  const socket: WebSocket =
    origin === undefined
      ? new WebSocket(url, protocols)
      : Reflect.construct(WebSocket, [url, { protocols, headers: { origin } }]);
  socket.binaryType = "arraybuffer";
  let aborted: (() => void) | undefined;
  try {
    const opened = await new Promise<boolean>((resolve, reject) => {
      socket.addEventListener("open", () => resolve(true), { once: true });
      socket.addEventListener("close", () => resolve(false), { once: true });
      aborted = () => reject(signal?.reason);
      signal?.addEventListener("abort", aborted, { once: true });
    });
    if (!opened || socket.protocol !== WEBSOCKET_SUBPROTOCOL) {
      socket.close();
      return undefined;
    }
  } catch (cause) {
    socket.close();
    throw cause;
  } finally {
    if (aborted !== undefined) {
      signal?.removeEventListener("abort", aborted);
    }
  }
  return transportOfWebSocket(socket);
}

/**
 * `socket`, open, as a Connection's transport: each message it receives is
 * one line to read, and each line written is one text message. Its close
 * ends the bytes to read; a write once it is closing fails.
 */
export function transportOfWebSocket(socket: WebSocket): Transport {
  const encoder = new TextEncoder();
  const newline = encoder.encode("\n");
  let closed = false;
  const readable = new ReadableStream<Uint8Array>({
    start(controller) {
      socket.addEventListener("message", (event) => {
        if (closed) {
          return;
        }
        const data: unknown = event.data;
        const bytes =
          typeof data === "string"
            ? encoder.encode(data)
            : data instanceof ArrayBuffer
              ? new Uint8Array(data)
              : undefined;
        if (bytes !== undefined) {
          controller.enqueue(bytes);
          controller.enqueue(newline);
        }
      });
      socket.addEventListener(
        "close",
        () => {
          if (!closed) {
            closed = true;
            controller.close();
          }
        },
        { once: true },
      );
    },
    cancel() {
      closed = true;
      socket.close();
    },
  });
  const decoder = new TextDecoder("utf-8", { fatal: true });
  let pending = "";
  const writable = new WritableStream<Uint8Array>({
    write(chunk) {
      if (socket.readyState !== WebSocket.OPEN) {
        throw new Error("the WebSocket to the Daemon is closed");
      }
      pending += decoder.decode(chunk, { stream: true });
      let end = pending.indexOf("\n");
      while (end !== -1) {
        socket.send(pending.slice(0, end));
        pending = pending.slice(end + 1);
        end = pending.indexOf("\n");
      }
    },
    close() {
      socket.close(1000);
    },
    abort() {
      socket.close();
    },
  });
  return { readable, writable };
}
