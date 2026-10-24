// "Needs you" as the Console holds it (spec #592; ADR 0031): one list of
// everything waiting for a person, read with `needs_you.list` and kept
// current by the lossless `needs_you.changed` on the event stream, which the
// store subscribes to before it lists, so no change between the two is
// lost: the changes that arrive while it lists are applied on top of the
// list's answer. A reconnect resubscribes after the last event read; one
// the engine can no longer resume answers `events.resync`, whose snapshot
// replaces the list. The shell holds one store, which both its side menu's
// count and the "Needs you" screen read.
import type { NeedsYouChange, NeedsYouItem } from "@goodfetch/collie-client/generated";
import type { ConsoleClient, StreamItem } from "#client/client.ts";

/** Where the list stands. */
export type NeedsYouState =
  | { readonly kind: "reading" }
  /** Every item, the longest waiting first. */
  | { readonly kind: "listed"; readonly items: readonly NeedsYouItem[] }
  /** The engine refused the list; its English message (ADR 0079). */
  | { readonly kind: "failed"; readonly message: string };

/** The list before its first answer. */
export const READING: NeedsYouState = { kind: "reading" };

/** Whether `data` is a change of "Needs you", as `needs_you.changed` carries one. */
function isChange(data: unknown): data is NeedsYouChange {
  if (typeof data !== "object" || data === null || !("change" in data)) {
    return false;
  }
  return (
    (data.change === "listed" && "item" in data) || (data.change === "removed" && "id" in data)
  );
}

/** `items` the longest waiting first, then by ID, as the engine orders them. */
function ordered(items: Iterable<NeedsYouItem>): readonly NeedsYouItem[] {
  return [...items].toSorted((left, right) =>
    left.since === right.since
      ? left.id.localeCompare(right.id)
      : left.since.localeCompare(right.since),
  );
}

/** A refused call's message. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** "Needs you" over one Client: an external store React reads with `useSyncExternalStore`. */
export class NeedsYouStore {
  readonly #client: ConsoleClient;
  readonly #listeners = new Set<() => void>();
  readonly #items = new Map<string, NeedsYouItem>();
  #state: NeedsYouState = READING;

  constructor(client: ConsoleClient) {
    this.#client = client;
  }

  /** The list now. */
  readonly snapshot = (): NeedsYouState => this.#state;

  /** Calls `changed` at each change of the list, until the function it answers is called. */
  readonly watch = (changed: () => void): (() => void) => {
    this.#listeners.add(changed);
    return () => this.#listeners.delete(changed);
  };

  /** Subscribes to the event stream, then lists; the function it answers stops both. */
  start(): () => void {
    const subscription = this.#client.subscribe("events");
    let pending: StreamItem[] | undefined = [];
    let stopped = false;
    const read = async (): Promise<void> => {
      for await (const item of subscription) {
        if (pending === undefined) {
          if (this.#apply(item)) {
            this.#publish();
          }
        } else {
          pending.push(item);
        }
      }
    };
    const list = async (): Promise<void> => {
      const items: NeedsYouItem[] = [];
      let after: string | undefined;
      try {
        do {
          const page = await this.#client.call(
            "needs_you.list",
            after === undefined ? {} : { after },
          );
          items.push(...page.items);
          after = page.next ?? undefined;
        } while (after !== undefined);
      } catch (error) {
        if (!stopped) {
          this.#state = { kind: "failed", message: messageOf(error) };
          this.#emit();
        }
        return;
      }
      if (stopped) {
        return;
      }
      this.#replace(items);
      for (const item of pending ?? []) {
        this.#apply(item);
      }
      pending = undefined;
      this.#publish();
    };
    void read();
    void list();
    return () => {
      stopped = true;
      subscription.close();
    };
  }

  /** Asks the engine to dismiss the item `id`; the change it publishes takes the item off the list. */
  async dismiss(id: string): Promise<boolean> {
    const { dismissed } = await this.#client.call("needs_you.dismiss", { item: id });
    return dismissed;
  }

  #replace(items: Iterable<NeedsYouItem>): void {
    this.#items.clear();
    for (const item of items) {
      this.#items.set(item.id, item);
    }
  }

  /** Applies `item` of the event stream, answering whether it changed the list. */
  #apply(item: StreamItem): boolean {
    if (item.kind === "resync") {
      this.#replace(item.snapshot.needs_you);
      return true;
    }
    if (item.event !== "needs_you.changed" || !isChange(item.data)) {
      return false;
    }
    const change = item.data;
    if (change.change === "listed") {
      this.#items.set(change.item.id, change.item);
    } else {
      this.#items.delete(change.id);
    }
    return true;
  }

  #publish(): void {
    this.#state = { kind: "listed", items: ordered(this.#items.values()) };
    this.#emit();
  }

  #emit(): void {
    for (const listener of this.#listeners) {
      listener();
    }
  }
}
