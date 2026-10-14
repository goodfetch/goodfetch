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
// never in the page: page functions take an explicit argument (#472).
// The page's events, routes and the data they carry are placeholders owned by #472; `run.records()` is #479's.

import type { Page } from "./lifecycle.js";
import type { CallOptions, Request, Response, UrlMatcher } from "./time.js";

/** A console message. Its full shape is #472's. */
export interface ConsoleMessage {
  readonly type: string;
  readonly text: string;
}

/** The page's observation events, under Playwright's names (S3). The full list is #472's. */
export interface PageEventMap {
  readonly console: ConsoleMessage;
  readonly request: Request;
  readonly response: Response;
  readonly requestfinished: Request;
  readonly popup: Page;
}

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

/** An intercepted request waiting for the program's answer. Its semantics are #472's. */
export interface Route {
  readonly request: Request;
  continue(options?: CallOptions): Promise<void>;
  abort(options?: CallOptions): Promise<void>;
  fulfill(response: { readonly status: number; readonly body?: string }, options?: CallOptions): Promise<void>;
}

/**
 * The page's events. There is no `off` and no `once`: close the Subscription that `on` returns, and
 * wait for one event with `waitForEvent` (S2).
 */
export interface PageEvents {
  /** Observes an event, subscribed when called. */
  on<K extends keyof PageEventMap>(event: K, listener: Listener<PageEventMap[K]>): Subscription;
  /** Waits for one event, armed when called (ADR 0080). Its predicate runs here and may use the program's variables. */
  waitForEvent<K extends keyof PageEventMap>(
    event: K,
    options?: CallOptions & { readonly predicate?: (event: PageEventMap[K]) => boolean },
  ): Promise<PageEventMap[K]>;
  /** Answers matching requests, registered when called. The handler runs on the program's event loop. */
  route(url: UrlMatcher, handler: (route: Route) => void | Promise<void>): Subscription;
}

/** The Run's sequences. */
export interface RunEvents {
  /** Placeholder for #479: the Run's Records. Their type is #469's; where the sequence starts is #479's. */
  records(): EventIterator<Readonly<Record<string, unknown>>>;
}
