// Lent pages (ADR 0089): `page.lend()` opens a CDP endpoint that shows only the page's Session, for
// a library in the same program, and one helper per supported library lends a page and connects that
// library with the safe options. The helpers are free functions of this package, named by ADR 0099,
// taking the page and then the program's own library object, so no SDK depends on Playwright or
// Puppeteer (G7). Only the SDKs lend; a Firefox or WebKit page raises `CapabilityUnavailableError`.

import type { Closeable, Page } from "./lifecycle.js";
import type { ProgramError, UnavailableError } from "./results.js";
import type { CallOptions } from "./time.js";

/**
 * A lent page's endpoint: a WebSocket under ADR 0017's rules, its token in `headers`, never in `url`.
 * The attach ends with `close()`, the library's `Browser.close`, its last connection closing, the
 * page closing or the Run ending; in a crawl handler, with the work item's lease (ADR 0003).
 */
export interface CDPEndpoint extends Closeable {
  url(): string;
  headers(): Readonly<Record<string, string>>;
}

/** Lending belongs to the page, so Collie carries it and Foxhound's handlers inherit it. */
export interface LendsPage {
  /** Throws `SessionReservedError` while another attach on the page's Session is open. */
  lend(options?: CallOptions): Promise<CDPEndpoint>;
}

/** The part of Playwright's `chromium` the helper uses, matched by shape (G7). */
export interface PlaywrightChromium<P> {
  connectOverCDP(
    endpointURL: string,
    options?: { readonly headers?: Record<string, string>; readonly noDefaults?: boolean },
  ): Promise<{ contexts(): { pages(): P[] }[] }>;
}

/** The part of Puppeteer the helper uses, matched by shape (G7). */
export interface PuppeteerLauncher<P> {
  connect(options: {
    readonly browserWSEndpoint: string;
    readonly headers?: Record<string, string>;
    readonly defaultViewport?: null;
  }): Promise<{ pages(): Promise<P[]> }>;
}

/**
 * Lends the page and connects Playwright to it with `noDefaults: true`, returning Playwright's page for
 * it. Closing Playwright's browser ends the attach and never closes the browser (ADR 0089).
 */
export declare function lendToPlaywright<P>(page: Page, chromium: PlaywrightChromium<P>, options?: CallOptions): Promise<P>;
/** Lends the page and connects Puppeteer to it with `defaultViewport: null`, returning Puppeteer's page for it. */
export declare function lendToPuppeteer<P>(page: Page, puppeteer: PuppeteerLauncher<P>, options?: CallOptions): Promise<P>;

/** A second `page.lend()` on a Session while one attach is open, or a lend of a Session a handler's attach reserves (ADR 0003). */
export declare class SessionReservedError extends ProgramError {
  readonly code: "session_reserved";
}
/** The attach was cut: the work item's lease ended, the page closed or the Run ended (ADR 0003). Lend the page again. */
export declare class AttachEndedError extends UnavailableError {
  readonly code: "attach_ended";
  readonly details: { readonly reason: "closed" | "lease" | "page_closed" | "run_ended" | (string & {}) };
}
