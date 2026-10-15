// Entry points and lifecycle (ADR 0078, decided in #465), with ADR 0095's packages: the values a
// connection hands out (Run, Session, Page) live in this Collie client package, and each Product's
// package adds its connection, `Collie` in `@goodfetch/collie` and `Foxhound` in `@goodfetch/foxhound`.
// The Run's start verbs, its ID, its end and its report are ADR 0102's (run.d.ts); Sessions, logins and secrets are ADR 0092's
// (sessions.d.ts); the page API is ADR 0085's (page.d.ts).
// Timeouts, waits and Limits come from #467 (time.d.ts); events and user code from #468 (concurrency.d.ts).

import type { AgentAPI } from "./agent.js";
import type { AIVerbs, AriaSnapshots, ElementReferences } from "./ai.js";
import type { Connection, RawCall } from "./catalog.js";
import type { Listener, PageEvents, RunEvents, Subscription } from "./concurrency.js";
import type { JSONObject, JSONValue } from "./data.js";
import type { DeclaresEffects } from "./effects.js";
import type { LendsPage } from "./lending.js";
import type { ModelProviderHandler, ReadsUsage } from "./models.js";
import type { KeyValueStore } from "./output.js";
import type { ProxyProviderHandler } from "./plugins.js";
import type { ProgramError, RefusedError } from "./results.js";
import type { PageAPI, PageCloseOptions, PageFunction, PageResult, RouteHandler } from "./page.js";
import type { RunControl, RunView } from "./run.js";
import type { Identities, PageStorage, PausesForHuman, ResolveSecret, SessionStateAPI, SessionSource } from "./sessions.js";
import type { ExplainedSetting, ExplainsSettings, NewPageOptions, SessionSettings, SettingKey, Settings } from "./settings.js";
import type { CallOptions, Limits, PageWaits, StartOptions, Timeout, URLMatcher } from "./time.js";

/** Which engine `connect()` reaches. `auto` follows ADR 0024: a Daemon when one is configured or running, otherwise an Attached engine. */
export type EngineChoice =
  | "auto"
  | "attached"
  | "daemon"
  | { readonly url: string; readonly token: string };

/** The engine a connection reached, reported for diagnosis (G6). */
export interface EngineInfo {
  readonly kind: "attached" | "daemon" | "remote";
  /** The engine's Product (ADR 0095): `foxhound` serves a Collie connection too. A newer one is kept (G3). */
  readonly product: "collie" | "foxhound" | (string & {});
  /** The engine's Product version (ADR 0090), reported, never compared. */
  readonly version: string;
  readonly protocolVersion: string;
  /** The project configuration file read, or `null` when none was found or `project` was `null` (ADR 0083). */
  readonly projectFile: string | null;
  /** The Goodfetch home the engine uses (`GOODFETCH_HOME`), or `null` for an Exposed Daemon or Goodfetch Cloud. */
  readonly home: string | null;
  /** The engine executable started (`GOODFETCH_COLLIE_ENGINE_PATH` or `GOODFETCH_FOXHOUND_ENGINE_PATH`, ADR 0090), or `null` when none was started. */
  readonly enginePath: string | null;
  /** The address reached (`GOODFETCH_URL` or `engine`), or `null` for an engine on this machine. `GOODFETCH_TOKEN` never appears. */
  readonly url: string | null;
}

/** The host's logger. `console`, pino and winston fit it. With none, the SDK is silent (G4). */
export interface Logger {
  debug(message: string, fields?: Readonly<Record<string, unknown>>): void;
  info(message: string, fields?: Readonly<Record<string, unknown>>): void;
  warn(message: string, fields?: Readonly<Record<string, unknown>>): void;
  error(message: string, fields?: Readonly<Record<string, unknown>>): void;
}

/** Options of `connect()`: its own, then every other setting in `settings` (ADR 0083). */
export interface ConnectOptions {
  /** Left at `auto`, `GOODFETCH_URL` fills it when set; a value written here wins over it. */
  readonly engine?: EngineChoice;
  readonly logger?: Logger;
  /** How long starting or finding the engine and `hello` may take. Default: the connect phase's setting. */
  readonly timeout?: Timeout;
  /** Limits on this Client connection's work (ADR 0041), lowered or set by the program itself. */
  readonly limits?: Limits;
  /** Settings for this connection and everything opened beneath it. */
  readonly settings?: Settings;
  /**
   * The environment map, read once here and never at import: the process's own when left out
   * (ADRs 0062, 0069). Only the deployment variables are read, and an unknown `GOODFETCH_` name fails
   * with `setting_unknown`. An Attached engine gets exactly this map as its environment.
   */
  readonly env?: Readonly<Record<string, string>>;
  /** The project configuration file. Left out: found upward from the working directory, as the CLI does. `null`: none. */
  readonly project?: string | null;
  /** Answers `caller:` Secret references from the program's own store, at the moment of use (ADR 0092). */
  readonly resolveSecret?: ResolveSecret;
  /**
   * Model providers answered by the program's own code, by the name Model references use, such as
   * `"mine/any-model"` for `{ mine: handler }`, from the Runs this connection opens (ADR 0101; ADR
   * 0033's SDK-handler form). A name taken by a built-in, a declared provider or a Plugin fails with
   * `setting_invalid`.
   */
  readonly modelProviders?: Readonly<Record<string, ModelProviderHandler>>;
  /**
   * Proxy providers answered by the program's own code, by the name settings choose them by (ADR
   * 0104; ADR 0033's SDK-handler form). The only one on the connection becomes the value of the
   * setting that chooses a Proxy provider, with the run-layer origin `registered`, which every other
   * run-layer value replaces and a lock beats; two fail `connect()` when nothing else sets the key.
   */
  readonly proxyProviders?: Readonly<Record<string, ProxyProviderHandler>>;
}

/** A value the program holds and closes: `await using`, or `close()` where `await using` is missing (Node 22). */
export interface Closeable extends AsyncDisposable {
  /** Idempotent. Closes children first; any later call on this value fails with the Error code `closed`. Its timeout is the close phase's. */
  close(options?: CallOptions): Promise<void>;
}

/** A Playwright or Puppeteer page, matched by shape so no third-party type is required (G7). */
export type BorrowablePage =
  | { context(): { newCDPSession(page: never): Promise<unknown> } }
  | { createCDPSession(): Promise<unknown> };

/** A Playwright or Puppeteer browser, matched by shape. */
export type BorrowableBrowser =
  | { newBrowserCDPSession(): Promise<unknown> }
  | { target(): unknown; disconnect(): Promise<void> };

/**
 * A page Goodfetch drives: a tab of a Session, opened by `session.newPage()` or by the page itself
 * (a popup). Closing it closes only a tab Goodfetch opened; on a Borrowed browser it only releases it.
 */
export interface Page
  extends Closeable,
    PageAPI,
    PageWaits,
    PageEvents,
    PageStorage,
    PausesForHuman,
    ExplainsSettings,
    AIVerbs,
    AriaSnapshots,
    ElementReferences,
    RawCall,
    AgentAPI,
    LendsPage {
  close(options?: PageCloseOptions): Promise<void>;
}

/** The Session's events. */
export interface SessionEventMap {
  /** A new page in the Session: one it opened, a popup, or a tab a link opened. */
  readonly page: Page;
}

/**
 * A Session of a Run: Playwright's context, under the glossary's name (ADR 0085), with its cookies and
 * login (ADR 0092). Closing it never closes a Borrowed browser. There is no active page.
 */
export interface Session extends Closeable, SessionStateAPI, ExplainsSettings, RawCall {
  newPage(options?: NewPageOptions): Promise<Page>;
  /** The Session's open pages, read without a round trip. */
  pages(): Page[];
  on<K extends keyof SessionEventMap>(event: K, listener: Listener<SessionEventMap[K]>): Subscription;
  /** Answers requests of every page of the Session, a popup's first request included; page routes run first. */
  route(url: URLMatcher, handler: RouteHandler): Subscription;
  /** Runs before the page's own scripts in every new document of the Session, marked in Provenance. */
  addInitScript<Arg extends JSONValue = JSONValue>(script: PageFunction<Arg, PageResult>, arg?: Arg): Subscription;
}

/** Options of opening a Session (ADR 0092). */
export interface NewSessionOptions extends CallOptions {
  /** `session.source` for this Session alone: where its login comes from (ADR 0056). Left out: the Run's or the connection's. */
  readonly source?: SessionSource;
  /**
   * ADR 0006's scripted login step: called with a page of this Session on its login page, where the
   * program's own code runs (ADR 0081). The Page check judges the page it leaves; it counts toward the
   * two recoveries a page gets, and an exception is chained to `LoggedOutError`.
   */
  readonly login?: (page: Page) => Promise<void>;
  readonly settings?: SessionSettings;
}

/**
 * A Run, always opened explicitly. `finish()` ends it `finished`; closing an unfinished Run stops it
 * with a Checkpoint, resumable by starting its ID again (ADR 0102).
 */
export interface Run extends Closeable, RunEvents, ExplainsSettings, ReadsUsage, RunControl, RawCall, DeclaresEffects {
  /** Chosen by the program at the start, or generated: it names this Run for good. Read without a round trip. */
  id(): string;
  /**
   * The Run's Key-value store, Collie's included (ADR 0103). Writes commit at once, or, through a
   * handler's `ctx.run`, are staged with its attempt.
   */
  readonly keyValueStore: KeyValueStore;
  /**
   * Opens a page in the Run's default Session, which the first call opens with the Run's settings and
   * every later call shares; `page.session()` reaches it. Once it is closed, this throws `ClosedError`.
   */
  newPage(options?: NewPageOptions): Promise<Page>;
  /** Opens a Session of its own. */
  newSession(options?: NewSessionOptions): Promise<Session>;
  /**
   * Borrows the user's own page (ADR 0056's `client_browser`). Ending the borrow never closes it.
   * The program's own driver keeps the page too, outside the engine's order of Gestures: await each
   * Collie call before that driver acts on the page again. An AI verb whose document a navigation it
   * did not start replaces throws `PageNavigatedDuringCallError` (ADR 0114); the driver's input
   * within one document is beyond what the engine sees.
   */
  borrow(page: BorrowablePage): Promise<Page>;
  /** Borrows the user's own browser, in its default context. Ending the borrow never closes it. */
  borrow(browser: BorrowableBrowser): Promise<Session>;
}

/** `run.borrow()` was given a Firefox or WebKit page, which exposes no CDP (ADRs 0056, 0089): borrow a Chromium page. */
export declare class BorrowRequiresCDPError extends ProgramError {
  readonly code: "borrow_requires_cdp";
}
/** Another Run already borrows this browser, judged on the browser the connection reaches, not on its address (ADR 0056). */
export declare class BrowserBorrowedError extends RefusedError {
  readonly code: "browser_borrowed";
}
