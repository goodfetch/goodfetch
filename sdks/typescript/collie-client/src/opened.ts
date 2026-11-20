// The values the events of one listener's call open (ADR 0081), such as the
// WebSockets of a page's `websocket`, whose own events the same call
// carries. Each value's listeners get its own events alone, in the call's
// order, each awaited before the next, as the call's own listener's are;
// one that throws is removed alone and its failure heard, and the call goes
// on. A value's `close` event is the last it gets.
import type { Listener, OpenedEvents, Opens, Subscription } from "./handle.ts";
import type { StreamedEvent } from "./stream.ts";

/** A listener of one of a value's own events. */
interface OwnListener {
  /** The event, as the engine names it, such as `web_socket.frame_received`. */
  readonly event: string;
  readonly listener: Listener<unknown>;
}

/** The values one call's events opened, and the listeners of each one's own events. */
export class Opened {
  /** What begins the name of each of an opened value's own events, such as `web_socket.`. */
  readonly #prefix: string;
  /** The name of the event after which an opened value gets nothing more, such as `web_socket.close`. */
  readonly #close: string;
  readonly #id: string;
  /** Reads an event's payload as the engine sent it. */
  readonly #read: (event: string, data: unknown) => unknown;
  /** Hears what a listener of `event` threw, once it was removed for it. */
  readonly #failed: (cause: unknown, event: string) => void;
  /** Each open value's listeners, by the ID its events name it by. */
  readonly #values = new Map<unknown, Set<OwnListener>>();

  constructor(
    opens: Pick<Opens<unknown>, "namespace" | "id">,
    read: (event: string, data: unknown) => unknown,
    failed: (cause: unknown, event: string) => void,
  ) {
    this.#prefix = `${opens.namespace}.`;
    this.#close = `${opens.namespace}.close`;
    this.#id = opens.id;
    this.#read = read;
    this.#failed = failed;
  }

  /** Opens the value `payload`, read, names, whose own events the call carries from now on. */
  open(payload: unknown): OpenedEvents {
    const listeners = new Set<OwnListener>();
    this.#values.set(this.#idIn(payload), listeners);
    return {
      listen<Payload>(event: string, listener: Listener<Payload>): Subscription {
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- engine payloads are not validated beyond `hello`'s protocol version (STYLE.md): only `event`'s payloads reach it.
        const own: OwnListener = { event, listener: listener as Listener<unknown> };
        listeners.add(own);
        const close = (): void => {
          listeners.delete(own);
        };
        return { close, [Symbol.dispose]: close };
      },
    };
  }

  /** Whether `event` is one of an opened value's own. */
  takes(event: StreamedEvent): boolean {
    return event.event.startsWith(this.#prefix);
  }

  /**
   * Hands `event` to the listeners of the value it names, one at a time,
   * in the order they were registered; never rejects. After the value's
   * `close`, it gets nothing more.
   */
  async deliver(event: StreamedEvent): Promise<void> {
    const payload = this.#read(event.event, event.data);
    const id = this.#idIn(payload);
    const listeners = this.#values.get(id);
    if (event.event === this.#close) {
      this.#values.delete(id);
    }
    if (listeners === undefined) {
      return;
    }
    // oxlint-disable-next-line unicorn/no-useless-spread -- a copy: a Set's own iteration would also visit a listener one of them registers for this very event.
    for (const own of [...listeners]) {
      // A listener an earlier one removed, through its Subscription, is skipped.
      if (own.event !== event.event || !listeners.has(own)) {
        continue;
      }
      try {
        await own.listener(payload, event.dropped ?? 0);
      } catch (error) {
        listeners.delete(own);
        this.#failed(error, event.event);
      }
    }
  }

  #idIn(payload: unknown): unknown {
    return typeof payload === "object" && payload !== null
      ? Reflect.get(payload, this.#id)
      : undefined;
  }
}
