// Sessions, logins, the Human pause and secrets in code (ADR 0092, decided in #476).
// - A Session opens from its Session source (ADR 0056): `run.newSession({ source })`. `run.newPage()`
//   opens pages in the Run's default Session, reached as `page.session()`.
// - A login is kept as a Saved identity (`session.saveIdentity`), never as a clear-text file; a lost
//   one is found by the Session's Page check and recovered by ADR 0006's ladder, or raises `logged_out`.
// - `page.pause()` is the Human pause: no clock runs while a person acts, in the real window or the
//   Live view, and it ends when its condition holds or the person says so.
// - A program names a secret by a Secret placeholder, `secret("name")` or `%name%` in an AI
//   instruction, which `secrets.<name>` binds to a Secret reference and the domains allowed to receive
//   it. The engine types or sends the value; the program never receives it, except what it answers for
//   a `caller:` reference (ADRs 0014, 0015).

import type { Locator } from "./page.js";
import type { PageError, ProgramError, RefusedError, SettingsLayer, UnavailableError } from "./results.js";
import type { CallOptions, Timeout, URLMatcher } from "./time.js";

// Secrets.

/**
 * A Secret reference, `provider:rest`: `env:QUOTES_PASSWORD`, `keychain:goodfetch/portal`,
 * `op://vault/item/field`, or `caller:portal` for `connect()`'s `resolveSecret`. It resolves in the
 * engine, at the moment of use: `env:` reads the engine's environment, which for an Attached engine is
 * `connect()`'s environment map (ADR 0083). A value that is not a reference is refused with
 * `setting_invalid`, so a raw password is never sent.
 */
export type SecretReference = `${string}:${string}`;

/**
 * A Secret placeholder: the name by which the program asks for a secret to be typed or sent. It holds
 * no value; `secrets.<name>` binds it, and the engine refuses an unbound one with `SecretUnboundError`.
 */
export declare class SecretPlaceholder {
  private constructor();
  readonly name: string;
}

/** The Secret placeholder `name`, which `%name%` also names in an AI instruction. */
export declare function secret(name: string): SecretPlaceholder;

/**
 * `secrets.<name>`: what a placeholder stands for and where it may go. `domains` are registrable
 * domains (ADR 0004), `*` only when written, and required; they are checked against the frame holding
 * the target field, or a request's URL. Set by the program's own code, configuration or an Operator
 * policy, at the connection, a Run or a Session, nearest winning; a Recipe declares the names it needs
 * and never binds them, and a Client acting for a model never sets one (`setting_locked`).
 */
export type SecretBinding =
  | {
      readonly reference: SecretReference;
      readonly domains: readonly string[];
      /** The reference holds a TOTP seed: the engine types the current code. A vault that makes codes itself needs no flag. */
      readonly totp?: boolean;
    }
  | {
      /** One of ADR 0006's email or SMS code sources, configured by name; a Human pause takes over when no code arrives. */
      readonly codeSource: string;
      readonly domains: readonly string[];
    };

/** `secrets.*`, keyed by placeholder name. */
export type SecretsSettings = Readonly<Record<string, SecretBinding>>;

/**
 * Header values a request or a route sends: a placeholder is resolved by the engine, checked against the
 * request's URL, and added at the engine's own interception, where no page script or Service Worker reads it.
 */
export type HeadersInput = Readonly<Record<string, string | SecretPlaceholder>>;

/** A JSON body a request sends, whose strings may be placeholders. */
export type JSONValueWithSecrets =
  | null
  | boolean
  | number
  | string
  | SecretPlaceholder
  | readonly JSONValueWithSecrets[]
  | { readonly [key: string]: JSONValueWithSecrets };

/** What `resolveSecret` is asked for. */
export interface SecretRequest {
  /** The reference's name after `caller:`. */
  readonly name: string;
  readonly runId: string;
  /** The registrable domain about to receive it, or `null` for a credential the engine uses itself. */
  readonly domain: string | null;
}

/**
 * Answers `caller:` references at the moment of use (ADR 0015's caller provider). It runs where the
 * program's code runs (ADR 0081); the value goes straight to the engine, and the SDK keeps no copy.
 * A throw fails the call with `SecretUnavailableError`.
 */
export type ResolveSecret = (request: SecretRequest) => Promise<string>;

// Where a Session's login comes from (ADR 0056).

/** A browser family, as ADR 0056 names them. */
export type BrowserName = "chrome" | "edge" | "brave" | "chromium" | "firefox" | "safari";

/**
 * `session.source`: exactly one kind (ADR 0056). `client_browser` is `run.borrow()`'s, never written.
 * Only the program's own code, configuration, an Operator policy or a person sets it; a Recipe never.
 */
export type SessionSource =
  | { readonly kind: "saved_identity"; readonly name: string }
  | {
      readonly kind: "state_file";
      /** Read on the program's machine and sent over the connection, so it works with a remote engine. */
      readonly path: string;
      readonly format?: "storage_state" | "cookies_txt" | "cdp_cookies" | "session_export";
      /** Opens a session export bundle (ADR 0015). */
      readonly key?: SecretReference;
      readonly domains?: readonly string[];
    }
  | { readonly kind: "cookie_header"; readonly header: string; readonly domain: string }
  | {
      readonly kind: "profile_directory";
      readonly path: string;
      readonly browser: BrowserName;
      readonly chromiumProfile?: string;
      readonly domains: readonly string[];
    }
  | { readonly kind: "installed_browser"; readonly browser: BrowserName; readonly chromiumProfile?: string; readonly domains: readonly string[] }
  | { readonly kind: "running_browser"; readonly browser: BrowserName; readonly domains: readonly string[] }
  | {
      readonly kind: "cdp_url";
      readonly url: string;
      readonly headers?: Readonly<Record<string, SecretReference>>;
      readonly domains?: readonly string[];
      /** Nothing else drives this browser, so secrets may be typed into it (ADR 0056). */
      readonly exclusive?: boolean;
    };

/** What a page looks like, matched in the engine: every field given must match. */
export interface PageMatch {
  /** A glob or a regular expression, in the Rust `regex` syntax (ADR 0081). */
  readonly url?: string | RegExp;
  /** CSS, or XPath starting with `//`, as `locator()` takes it. */
  readonly selector?: string;
  readonly text?: string | RegExp;
}

/**
 * `session.page_check` (ADR 0006): every page of the Session is judged against it. A login page is a
 * lost login, which recovery tries to bring back; a page that is neither expected nor a login page
 * fails with `page_check_failed`. Without one, the built-in detector applies and pages are weakly checked.
 */
export interface PageCheck {
  readonly loginPage?: PageMatch;
  readonly expectedPage?: PageMatch;
}

/** `session.http_credentials`: sent only to a listed domain that challenges, never on a browser other code may read. A Recipe and a Client acting for a model never set it. */
export interface HTTPCredentials {
  readonly username: string;
  readonly password: SecretReference;
  readonly domains: readonly string[];
}

/** `session.*`. */
export interface SessionLoginSettings {
  readonly source?: SessionSource;
  readonly pageCheck?: PageCheck | null;
  readonly recovery?: {
    /** `session.recovery.human_pause`: whether ADR 0006's ladder may end in a Human pause; `true` by default. */
    readonly humanPause?: boolean;
    /**
     * `session.recovery.agent`: whether the ladder's AI agent login step runs, the built-in Agent with the
     * Session's Secret placeholders; `true` by default, skipped with no model and no login Plan (ADR 0100).
     */
    readonly agent?: boolean;
  };
  readonly httpCredentials?: readonly HTTPCredentials[];
}

/**
 * `proxy.*`: part of a Session's identity, one proxy per Session (ADR 0021), with Playwright's fields.
 * Pools by kind and provider adapters are this family's too; area 8's spec names their keys. A Recipe and a
 * Client acting for a model never set `server` or the credentials; a Recipe may set pool kinds.
 */
export interface ProxySettings {
  readonly server?: string | null;
  /** Comma-separated domains that skip the proxy. */
  readonly bypass?: string;
  readonly username?: string;
  readonly password?: SecretReference;
}

/** `browser.permissions`, under Playwright's names. A sample: the SDK lists every one. */
export type Permission =
  | "clipboard-read"
  | "clipboard-write"
  | "geolocation"
  | "notifications"
  | "camera"
  | "microphone"
  | "local-network-access";

// Cookies and Saved identities.

/** A cookie, with Playwright's fields. `expires` is a `Date`, the SDK's point-in-time type, or `null` for a session cookie. */
export interface Cookie {
  readonly name: string;
  readonly value: string;
  readonly domain: string;
  readonly path: string;
  readonly expires: Date | null;
  readonly httpOnly: boolean;
  readonly secure: boolean;
  readonly sameSite: "Strict" | "Lax" | "None";
  readonly partitionKey?: string;
}

/** A cookie to add: by `url`, or by `domain` and `path`, as Playwright takes it. */
export type CookieInput = {
  readonly name: string;
  readonly value: string;
  readonly expires?: Date | null;
  readonly httpOnly?: boolean;
  readonly secure?: boolean;
  readonly sameSite?: "Strict" | "Lax" | "None";
  readonly partitionKey?: string;
} & ({ readonly url: string } | { readonly domain: string; readonly path: string });

export interface ClearCookiesFilter {
  readonly name?: string | RegExp;
  readonly domain?: string | RegExp;
  readonly path?: string | RegExp;
}

/** A Saved identity (ADR 0035), as `collie.identities.list()` and `session.saveIdentity()` return it. */
export interface SavedIdentity {
  readonly name: string;
  /** Where its login came from: an import's source kind, or `session` for a login made in a Session. */
  readonly source: SessionSource["kind"] | "session" | (string & {});
  readonly accessMode: "standard" | "identified" | "stealth" | (string & {});
  readonly savedAt: Date;
  /** The earliest cookie expiry per registrable domain: cookies expire then, which says nothing of the site's own session (ADR 0056). */
  readonly cookiesExpire: Readonly<Record<string, Date>>;
  /** An import on direct egress, or a login saved from a Session that came from one: usable only on this machine (ADR 0056). */
  readonly boundToMachine: boolean;
}

/** Exports a Saved identity as ADR 0015's `age`-encrypted bundle, to a public key or a passphrase. */
export type ExportIdentityOptions = CallOptions & { readonly path: string } & (
    | { readonly recipient: string }
    | { readonly passphrase: SecretReference }
  );

/** The install's Saved identities, reached as `collie.identities` or `foxhound.identities`. No SDK writes a clear-text `storageState` (ADR 0056). */
export interface Identities {
  list(options?: CallOptions): Promise<SavedIdentity[]>;
  /** Imports a login into a Saved identity, as `goodfetch-collie identity import` does. */
  import(name: string, source: SessionSource, options?: CallOptions): Promise<SavedIdentity>;
  delete(name: string, options?: CallOptions): Promise<void>;
  /** A bundle of one bound to its machine keeps the binding: used on another machine, it is refused with `session_bound_to_machine` (ADR 0056). */
  export(name: string, options: ExportIdentityOptions): Promise<void>;
}

/** A Session's cookies and keeping its identity, beside its page members (ADR 0085). */
export interface SessionStateAPI {
  /** Playwright's `context.cookies()`. Offered in the SDKs only, never on the CLI, MCP or Code mode (ADRs 0011, 0014). */
  cookies(urls?: string | readonly string[], options?: CallOptions): Promise<Cookie[]>;
  /** Kept as session state, encrypted in run state and never logged (ADR 0015). Outside the source's `domains`, refused. */
  addCookies(cookies: readonly CookieInput[], options?: CallOptions): Promise<void>;
  clearCookies(filter?: ClearCookiesFilter, options?: CallOptions): Promise<void>;
  /**
   * Keeps this Session's identity, cookies and storage as a Saved identity (ADR 0035), replacing one
   * of that name, recorded in the Audit log. A Session whose cookies live outside run state (a Borrowed
   * browser, a Profile directory used in place) throws `SessionNotSavableError`.
   */
  saveIdentity(name: string, options?: CallOptions): Promise<SavedIdentity>;
}

/** One item of a page's storage. */
export interface WebStorageItem {
  readonly name: string;
  readonly value: string;
}

/**
 * A page's `localStorage` or `sessionStorage` for its current origin, Playwright's `WebStorage` (1.61).
 * Offered in the SDKs only (ADRs 0011, 0014); outside a bound source's `domains`, refused. A value set
 * is session state; a Saved identity keeps `localStorage`, and `sessionStorage` travels only in an export bundle.
 */
export interface WebStorage {
  items(options?: CallOptions): Promise<WebStorageItem[]>;
  getItem(name: string, options?: CallOptions): Promise<string | null>;
  setItem(name: string, value: string, options?: CallOptions): Promise<void>;
  removeItem(name: string, options?: CallOptions): Promise<void>;
  clear(options?: CallOptions): Promise<void>;
}

/** A page's storage, under Playwright's names. */
export interface PageStorage {
  readonly localStorage: WebStorage;
  readonly sessionStorage: WebStorage;
}

// The Human pause.

/** Options of `page.pause()`. */
export interface PauseOptions extends CallOptions {
  /** Shown in the Console's "Needs you", the Notification, the Live view, and logged at info. */
  readonly reason?: string;
  /**
   * Ends the pause by itself when it holds: a locator visible, or a URL matched as `waitForURL`
   * matches it. A person's "Done" is accepted only while it holds.
   */
  readonly until?: Locator | URLMatcher;
  /** `timeout.human_pause`: 24 hours by default; `null` waits without limit. No other clock runs meanwhile. */
  readonly timeout?: Timeout;
}

/** How a Human pause ended. A person's "Decline" throws `HumanPauseDeclinedError` instead. */
export interface HumanPauseResult {
  readonly resolvedBy: "condition" | "person" | (string & {});
  /** The person who answered, as the Console or the CLI knows them; `null` when the condition ended it. */
  readonly person: string | null;
  /** How long it waited, in milliseconds. */
  readonly waited: number;
}

/** The Human pause on a page. */
export interface PausesForHuman {
  /**
   * Playwright's `page.pause()`, with the Human pause's meaning: the program waits while a person acts
   * in this page, in its window when it is headful on their desktop, else in the Live view on any
   * device. A paused page takes no Gesture: its later Gestures wait; reads and waits run. No enclosing
   * timeout counts its time; Usage still counts toward Limits. Cancelling the call ends it.
   */
  pause(options?: PauseOptions): Promise<HumanPauseResult>;
}

// Error codes. Each names the placeholder and the reference, never a value.

export interface SecretUnboundDetails {
  readonly placeholder: string;
  /** The names the scope binds. */
  readonly bound: readonly string[];
}
/** No scope binds this placeholder. */
export declare class SecretUnboundError extends ProgramError {
  readonly code: "secret_unbound";
  readonly details: SecretUnboundDetails;
}

export interface SecretUnavailableDetails {
  readonly placeholder: string | null;
  readonly reference: string;
  /** The provider asked, such as `env`, `keychain` or `caller`. */
  readonly provider: string;
  /** Which failure: the code is not retryable. */
  readonly failure: "missing" | "locked" | "unreachable" | "resolver_failed" | (string & {});
}
/** The provider gave no value: a missing variable, a locked keychain, an unreachable vault, or `resolveSecret` missing or throwing. */
export declare class SecretUnavailableError extends UnavailableError {
  readonly code: "secret_unavailable";
  readonly details: SecretUnavailableDetails;
}

export interface SecretDomainNotAllowedDetails {
  readonly placeholder: string;
  /** The domain of the frame holding the field, or of the request. */
  readonly domain: string;
  readonly allowed: readonly string[];
}
/** The field's frame or the request is outside the binding's `domains`. */
export declare class SecretDomainNotAllowedError extends RefusedError {
  readonly code: "secret_domain_not_allowed";
  readonly details: SecretDomainNotAllowedDetails;
}

export interface SecretInputRefusedDetails {
  readonly placeholder: string;
  /**
   * `shared_browser`: other code also drives this Borrowed browser (ADR 0056); `cdp_attached`: a raw CDP
   * session is attached (ADR 0017); `lent_page`: the page is lent to another program (ADR 0089).
   */
  readonly reason: "shared_browser" | "cdp_attached" | "lent_page" | (string & {});
}
/** No secret is typed into or sent from a browser other code may read, and no HTTP authentication challenge is answered there. */
export declare class SecretInputRefusedError extends RefusedError {
  readonly code: "secret_input_refused";
  readonly details: SecretInputRefusedDetails;
}

/** One step of ADR 0006's recovery ladder and why it did not bring the login back. */
export interface RecoveryStep {
  readonly step: "refresh" | "reimport" | "login" | "agent_login" | "human_pause" | (string & {});
  readonly outcome: "failed" | "skipped" | "declined" | "timed_out" | (string & {});
  readonly reason: string;
}
export interface LoggedOutDetails {
  readonly steps: readonly RecoveryStep[];
}
/**
 * The Session's Page check found a login page and recovery did not bring the login back, or was off.
 * An exception the `login` function threw is chained as the cause.
 */
export declare class LoggedOutError extends PageError {
  readonly code: "logged_out";
  readonly details: LoggedOutDetails;
}

export interface HumanPauseDeclinedDetails {
  readonly person: string | null;
  readonly note: string | null;
}
/** A person declined the step `page.pause()` asked for. */
export declare class HumanPauseDeclinedError extends RefusedError {
  readonly code: "human_pause_declined";
  readonly details: HumanPauseDeclinedDetails;
}

export interface SessionNotSavableDetails {
  readonly source: SessionSource["kind"] | "client_browser" | (string & {});
}
/** The Session's cookies live outside run state; `fix` names the import of its source. */
export declare class SessionNotSavableError extends ProgramError {
  readonly code: "session_not_savable";
  readonly details: SessionNotSavableDetails;
}

// Opening a Session on a `cdp_url`, and the browser build a Session launches (ADR 0115, decided in
// #534): one code per remedy, raised by the call that opens the Session or launches the browser
// (`newSession`, the default Session's first `newPage`, a crawl request's escalation), never by
// `connect()`. An address that never answers is `deadline_exceeded`; a `headers` reference that resolves
// to nothing is `secret_unavailable`.

export interface CDPURLDetails {
  /** The address, without its query string or any credentials in it. */
  readonly url: string;
  /** The settings layer that gave the address (ADR 0083). */
  readonly layer: SettingsLayer;
}
export interface CDPURLUnreachableDetails extends CDPURLDetails {
  readonly failure: "name_not_resolved" | "connection_refused" | "connection_reset" | "tls_failed" | (string & {});
}
/** Nothing at the address answered; `fix` says to start the browser with remote debugging on that port, or correct the address. Retryable. */
export declare class CDPURLUnreachableError extends UnavailableError {
  readonly code: "cdp_url_unreachable";
  readonly details: CDPURLUnreachableDetails;
}
/** What answered at a `cdp_url` that is not a browser, also in `observed`. */
export interface CDPURLAnswer {
  readonly kind: "http" | "websocket_refused" | "cdp_target" | (string & {});
  readonly status: number | null;
  readonly contentType: string | null;
  /** For `cdp_target`: `page`, `node`, `worker`, or `browser` for a product the engine does not drive. */
  readonly targetType: string | null;
  readonly product: string | null;
}
export interface CDPURLNotBrowserDetails extends CDPURLDetails {
  readonly answered: CDPURLAnswer;
  /** The browser endpoint the same host's `/json/version` lists, which `fix` names; `null` when none. */
  readonly browserEndpoint: string | null;
}
/** Something answered, but no browser speaks CDP there. */
export declare class CDPURLNotBrowserError extends ProgramError {
  readonly code: "cdp_url_not_browser";
  readonly details: CDPURLNotBrowserDetails;
}
export interface CDPURLUnauthorizedDetails extends CDPURLDetails {
  readonly status: 401 | 403;
  /** Each header sent, by name, with its Secret reference; never a value. */
  readonly headers: Readonly<Record<string, SecretReference>>;
}
/** The address answered 401 or 403; `fix` points to the Secret references in `headers`. */
export declare class CDPURLUnauthorizedError extends RefusedError {
  readonly code: "cdp_url_unauthorized";
  readonly details: CDPURLUnauthorizedDetails;
}
export interface CDPURLNotAllowedDetails extends CDPURLDetails {
  /** ADR 0056's rule: a private range, or loopback where it is not allowed. */
  readonly rule: "private_range" | "loopback" | (string & {});
}
/** ADR 0056's address rules refused the address before any connection; `fix` names Operator policy or user or project configuration. */
export declare class CDPURLNotAllowedError extends RefusedError {
  readonly code: "cdp_url_not_allowed";
  readonly details: CDPURLNotAllowedDetails;
}

export interface PinnedBy {
  readonly product: "collie" | "foxhound" | (string & {});
  readonly version: string;
}
/** A pinned download of the Browser cache: a browser build, Lightpanda, Playwright's driver, the Clean-up rules, an engine executable. */
export interface PinnedArtifact {
  /** Such as `chromium`, `camoufox` or `lightpanda`. */
  readonly name: string;
  /** The build ID or version pinned. */
  readonly version: string;
  /** The Product and Product version that pin it (ADR 0098). */
  readonly pinnedBy: PinnedBy;
  /** The Browser cache's path. */
  readonly cachePath: string;
}
export interface BrowserNotInstalledDetails {
  /** The pinned build, or `null` for a browser named by channel or path. */
  readonly build: PinnedArtifact | null;
  /** The channel or path looked for, when a setting named one (ADR 0024). */
  readonly channel: string | null;
  readonly path: string | null;
}
/**
 * The build a Session needs is not in a Browser cache the engine cannot write, or a browser named by
 * channel or path is absent; `fix` is the Product's install command for that build. Not retryable.
 */
export declare class BrowserNotInstalledError extends UnavailableError {
  readonly code: "browser_not_installed";
  readonly details: BrowserNotInstalledDetails;
}
export interface ArtifactUnavailableOfflineDetails {
  readonly artifact: PinnedArtifact;
}
/** The `offline` setting refused a pinned download (ADR 0043); `limit` names `offline` and its layer. */
export declare class ArtifactUnavailableOfflineError extends RefusedError {
  readonly code: "artifact_unavailable_offline";
  readonly details: ArtifactUnavailableOfflineDetails;
}
export interface ArtifactDownloadFailedDetails {
  readonly artifact: PinnedArtifact;
  /** The host it came from, never a full URL. */
  readonly host: string;
  readonly failure: "network" | "http_status" | "digest_mismatch" | (string & {});
  /** For `http_status`. */
  readonly status: number | null;
}
/** A pinned download on first use failed after the engine's own attempts. Retryable. */
export declare class ArtifactDownloadFailedError extends UnavailableError {
  readonly code: "artifact_download_failed";
  readonly details: ArtifactDownloadFailedDetails;
}
