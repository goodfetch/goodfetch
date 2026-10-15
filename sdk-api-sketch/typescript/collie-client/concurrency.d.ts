// Concurrency, events and user code (ADR 0081, decided in #468).
// TypeScript is async only. A connection belongs to the JavaScript realm that opened it: a
// `worker_threads` worker opens its own. Any number of calls may be in flight on any value; the SDK
// keeps no queue (K04), each call goes to the engine when it is made, and the engine starts a page's
// calls in arrival order (ADR 0080).
//
// The Operation catalog declares each event's kind, so each event has one shape:
// - side activity on a page is observed with a listener: `page.on(event, listener)` returns a Subscription;
// - a sequence the program reads to its end is an EventIterator, read with `for await`.
// Both subscribe when the method is called, as waiters arm when called, so nothing slips through
// before the first event. A request that needs the program's answer (a route here; a crawl handler,
// a dialog, a Hook in code elsewhere) is a function the program registers, run on its event loop; the
// engine sends no more at once than the concurrency setting allows.
//
// URL globs and regular expressions run in the engine, in the Rust `regex` syntax: no look-around or
// backreferences, and linear time. A `RegExp` using either fails at the call with a `ProgramError`
// naming the construct. A predicate runs here, on the events the engine sends while it is armed, and
// never in the page: page functions take an explicit argument (ADR 0085).
// The page's events, routes, dialogs and the data they carry are ADR 0085's (page.d.ts). A crawl's
// Records are read with `run.records(dataset)` on its `CrawlRun` (ADR 0103, crawl.d.ts).

import type { DialogHandler, PageEventMap } from "./page.js";
import type { CallOptions } from "./time.js";

/**
 * A listener. It gets its events one at a time, in order: a promise it returns is awaited
 * before it gets the next one, so lossless events slow their producer and telemetry coalesces
 * (ADR 0024). If it throws, it is removed, and the next call on the value it was registered on, or
 * the `close()` that closes that value, its own or a parent's, throws `ListenerFailedError` with
 * the exception as `cause`.
 */
export type Listener<T> = (event: T) => void | Promise<void>;

/** A registered listener or route. It closes like any value (ADR 0078), synchronously, since nothing waits. */
export interface Subscription extends Disposable {
  /** Idempotent. Unsubscribes without waiting for the engine. */
  close(): void;
}

/**
 * A sequence the program reads to its end with `for await`. Leaving the loop (`break`, `return`, a
 * throw) or `close()` unsubscribes without waiting. One reader each: a second reader calls the method
 * again and gets its own subscription. While open, an unread lossless sequence slows its Run
 * (ADR 0024); a telemetry sequence keeps only the latest value.
 */
export interface EventIterator<T> extends AsyncIterableIterator<T>, Disposable {
  /** Idempotent. Unsubscribes without waiting for the engine. */
  close(): void;
}

/**
 * The page's events. There is no `off` and no `once`: close the Subscription that `on` returns, and
 * wait for one event with `waitForEvent` (S2).
 */
export interface PageEvents {
  /** Observes an event, subscribed when called. */
  on<K extends keyof PageEventMap>(event: K, listener: Listener<PageEventMap[K]>): Subscription;
  /**
   * Answers dialogs, registered when called: with none, or none that answers, a dialog gets
   * Playwright's default, dismissed, or accepted for `beforeunload` (ADR 0085).
   */
  on(event: "dialog", handler: DialogHandler): Subscription;
  /** Waits for one event, armed when called (ADR 0080). Its predicate runs here and may use the program's variables. */
  waitForEvent<K extends keyof PageEventMap>(
    event: K,
    options?: CallOptions & { readonly predicate?: (event: PageEventMap[K]) => boolean },
  ): Promise<PageEventMap[K]>;
}

/**
 * The Run's sequences. Its progress is `run.progress()` (ADR 0102); a crawl's outcomes and Records
 * are read from its `CrawlRun`, since only Foxhound has Datasets (ADRs 0084, 0102, 0103).
 */
export interface RunEvents {}
