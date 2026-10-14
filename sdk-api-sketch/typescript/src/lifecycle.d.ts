// Entry points and lifecycle (ADR 0078, decided in #465).
// The Run's start verbs, Sessions and the page API are placeholders owned by #478, #476 and #472.
// Timeouts, waits and Limits come from #467 (time.d.ts); events and user code from #468 (concurrency.d.ts).

import type { PageEvents, RunEvents } from "./concurrency.js";
import type { ExplainedSetting, ExplainsSettings, NewPageOptions, SessionSettings, SettingKey, Settings } from "./settings.js";
import type { CallOptions, Limits, PageWaits, StartOptions, Timeout } from "./time.js";

/** Which engine `connect()` reaches. `auto` follows ADR 0024: a Daemon when one is configured or running, otherwise an Attached engine. */
export type EngineChoice =
  | "auto"
  | "attached"
  | "daemon"
  | { readonly url: string; readonly token: string };

/** The engine a connection reached, reported for diagnosis (G6). */
export interface EngineInfo {
  readonly kind: "attached" | "daemon" | "remote";
  readonly version: string;
  readonly protocolVersion: string;
  /** The project configuration file read, or `null` when none was found or `project` was `null` (ADR 0083). */
  readonly projectFile: string | null;
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

/** A page Goodfetch drives. Its API is #472's. Closing it closes only a tab Goodfetch opened. */
export interface Page extends Closeable, PageWaits, PageEvents, ExplainsSettings {}

/** A Session of a Run. Its API is #476's. Closing it never closes a Borrowed browser. */
export interface Session extends Closeable, ExplainsSettings {
  newPage(options?: NewPageOptions): Promise<Page>;
}

/** A Run, always opened explicitly. Closing an unfinished Run stops it with a Checkpoint, resumable by its ID. */
export interface Run extends Closeable, RunEvents, ExplainsSettings {
  readonly id: string;
  newPage(options?: NewPageOptions): Promise<Page>;
  /** Placeholder: opening a Session is #476's; its settings are #470's. */
  newSession(options?: CallOptions & { readonly settings?: SessionSettings }): Promise<Session>;
  /** Borrows the user's own page (ADR 0056's `client_browser`). Ending the borrow never closes it. */
  borrow(page: BorrowablePage): Promise<Page>;
  /** Borrows the user's own browser, in its default context. Ending the borrow never closes it. */
  borrow(browser: BorrowableBrowser): Promise<Session>;
}

/** Collie, reached from the connection without I/O. */
export interface Collie {
  /** Placeholder start verb: its spelling is #478's; `timeout` and `limits` are #467's. */
  start(options?: StartOptions): Promise<Run>;
}

/** Foxhound, reached from the connection without I/O. Its crawl API is #477's. */
export interface Foxhound {}

/** The engine connection: the entry point. Each one is independent, and many may be open in one process. */
export declare class Goodfetch implements Closeable, ExplainsSettings {
  private constructor();
  /** Starts or finds the engine and completes `hello` before returning. */
  static connect(options?: ConnectOptions): Promise<Goodfetch>;
  readonly engine: EngineInfo;
  readonly collie: Collie;
  readonly foxhound: Foxhound;
  /** Cancels in-flight calls, closes every Run it opened and ends every borrow; stops an Attached engine, only disconnects from a Daemon. */
  close(options?: CallOptions): Promise<void>;
  [Symbol.asyncDispose](): Promise<void>;
  explainSettings(keys?: readonly SettingKey[], options?: CallOptions): Promise<readonly ExplainedSetting[]>;
}
