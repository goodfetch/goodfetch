// Collie's page API (ADR 0085, decided in #472).
// Playwright's page API (ADR 0019) under its names where the meaning is the same, with the guard's
// guarantees:
// - Pages, Sessions and frames are values. The Session is Playwright's context: pages open with
//   `session.newPage()` or arrive as popups, and frames are reached through FrameLocators. There is
//   no active page, no frame pointer and no element handle: a Locator is resolved again at each use.
// - Every action, and every read of one element, waits through the guard until exactly one element
//   matches and is actionable, or its deadline passes; a match still ambiguous then throws
//   `TargetNotUniqueError` listing every match.
// - The engine runs a page's gestures (actions, keyboard, mouse and touch input, navigations) one at a
//   time in arrival order; reads, waits, `evaluate` and answers to routes and dialogs run concurrently.
// - A page function is JavaScript run in the page, with one explicit argument; values cross as JSON.
// - Routes and dialog handlers answer requests: later ones first, the first to settle wins, one that
//   settles nothing falls through, and past the last one a request goes to the network and a dialog
//   gets Playwright's default, so a handler that answers nothing never stalls the page.
// - A gesture that waits on the program (a locator handler's turn before an attempt, or a route's
//   answer to a request it waits on, after its input) gives up its page's turn, only between its own
//   inputs, so a handler's own gestures run and no two gestures' inputs interleave. An open dialog
//   blocks input, so a dialog handler answers before it acts on the page.
// Waits come from ADR 0080 (time.d.ts), events and Subscriptions from ADR 0081 (concurrency.d.ts).

import type { AIVerbs, AriaSnapshots } from "./ai.js";
import type { RawCall } from "./catalog.js";
import type { Listener, Subscription } from "./concurrency.js";
import type { JSONObject, JSONValue } from "./data.js";
import type { Closeable, Page, Session } from "./lifecycle.js";
import type { PageError, ProgramError, RefusedError, UnavailableError } from "./results.js";
import type { HeadersInput, JSONValueWithSecrets, SecretPlaceholder } from "./sessions.js";
import type { CallOptions, Count, LoadState, URLMatcher } from "./time.js";

// Locators.

/** ARIA roles, as `getByRole` takes them. A sample: the SDK lists every ARIA role. */
export type AriaRole =
  | "alert"
  | "button"
  | "checkbox"
  | "combobox"
  | "dialog"
  | "heading"
  | "link"
  | "listbox"
  | "listitem"
  | "menuitem"
  | "option"
  | "radio"
  | "row"
  | "searchbox"
  | "tab"
  | "table"
  | "textbox";

/**
 * Text to match. A string matches as a case-insensitive substring after whitespace is normalized,
 * as in Playwright, or exactly with `exact: true`. A `RegExp` runs in the engine, in the Rust `regex`
 * syntax (ADR 0081).
 */
export type TextMatch = string | RegExp;

export interface TextOptions {
  readonly exact?: boolean;
}

export interface GetByRoleOptions extends TextOptions {
  /** The accessible name. */
  readonly name?: TextMatch;
  readonly checked?: boolean;
  readonly disabled?: boolean;
  readonly expanded?: boolean;
  readonly includeHidden?: boolean;
  readonly level?: number;
  readonly pressed?: boolean;
  readonly selected?: boolean;
}

/** Declarative filters, resolved by the engine: no host callback ever runs in the page. */
export interface FilterOptions {
  readonly hasText?: TextMatch;
  readonly hasNotText?: TextMatch;
  readonly has?: Locator;
  readonly hasNot?: Locator;
  readonly visible?: boolean;
}

/** The locator constructors that pages, frame locators and locators share. None does I/O. */
export interface Locates {
  /**
   * CSS, or XPath when the string starts with `//`, `..` or `xpath=`. CSS pierces open shadow
   * roots; XPath does not. Playwright's other selector engines (`text=`, `role=`, `internal:`), `>>`
   * chains and its CSS extensions (`:has-text()`, `:text()`, `:visible`, `:nth-match()`) fail with
   * `SelectorInvalidError`, whose fix names the method that replaces them.
   */
  locator(selector: string, options?: FilterOptions): Locator;
  getByRole(role: AriaRole, options?: GetByRoleOptions): Locator;
  getByText(text: TextMatch, options?: TextOptions): Locator;
  getByLabel(text: TextMatch, options?: TextOptions): Locator;
  getByPlaceholder(text: TextMatch, options?: TextOptions): Locator;
  getByAltText(text: TextMatch, options?: TextOptions): Locator;
  getByTitle(text: TextMatch, options?: TextOptions): Locator;
  /** Matches the attribute named by the setting `locator.test_id_attribute`, `data-testid` by default. */
  getByTestId(testId: TextMatch): Locator;
  /**
   * Enters an iframe matched by `selector`, same-origin or cross-origin alike. Without a selector,
   * the search starts in every frame of the page; the rest of the locator resolves inside one frame.
   */
  frameLocator(selector?: string): FrameLocator;
}

/** An iframe's content, resolved again at each use. Strict: its `iframe` must be unique. */
export interface FrameLocator extends Locates {
  /** The `iframe` element itself. */
  owner(): Locator;
}

export type Modifier = "Alt" | "Control" | "ControlOrMeta" | "Meta" | "Shift";

export interface Position {
  readonly x: number;
  readonly y: number;
}

export interface BoundingBox {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/** Options of a guarded action. Its timeout is the action phase's (ADR 0080). */
export interface ActionOptions extends CallOptions {
  /**
   * Skips the guard's actionability checks and its replay, keeps uniqueness, and marks the action
   * unguarded in Provenance: the setting `action.force` for this call alone (ADR 0023).
   */
  readonly force?: boolean;
}

export interface PointerOptions extends ActionOptions {
  readonly modifiers?: readonly Modifier[];
  /** Relative to the element's top-left corner; its visible centre by default. */
  readonly position?: Position;
}

export interface ClickOptions extends PointerOptions {
  readonly button?: "left" | "right" | "middle";
  readonly clickCount?: number;
  /** Milliseconds between pressing and releasing. */
  readonly delay?: number;
}

export interface KeyOptions extends ActionOptions {
  /** Milliseconds between key presses. */
  readonly delay?: number;
}

/** A file held in memory. */
export interface FilePayload {
  readonly name: string;
  readonly mimeType: string;
  readonly buffer: Uint8Array;
}

/**
 * Files to upload: paths on the program's machine, which the SDK reads and streams over the
 * connection, or files in memory. An empty list clears the input.
 */
export type InputFiles = string | FilePayload | readonly string[] | readonly FilePayload[];

export type SelectOptionValue =
  | string
  | { readonly value: string }
  | { readonly label: string }
  | { readonly index: number };

/** A page function sees only its argument; the page runs it in its main world unless asked otherwise. */
export interface EvaluateOptions extends CallOptions {
  /**
   * `main`, the default, sees the page's own globals and is marked in Provenance (ADR 0019);
   * `isolated` runs where the page can neither see nor hook it.
   */
  readonly world?: "main" | "isolated";
}

/** What a page function may return: JSON, or nothing. */
export type PageResult = JSONValue | undefined | void;

/**
 * JavaScript run in the page: a function, whose source is sent and which sees only its argument,
 * or source text, a function if it parses as one and otherwise an expression. Values cross as JSON
 * both ways; anything else throws `PageValueInvalidError` naming its path.
 */
export type PageFunction<Arg extends JSONValue, R extends PageResult> = string | ((arg: Arg) => R | Promise<R>);
/** A page function given the locator's element first. */
export type ElementFunction<Arg extends JSONValue, R extends PageResult> =
  | string
  | ((element: Element, arg: Arg) => R | Promise<R>);
/** A page function given every matching element first. */
export type ElementsFunction<Arg extends JSONValue, R extends PageResult> =
  | string
  | ((elements: Element[], arg: Arg) => R | Promise<R>);

export interface ScreenshotOptions extends CallOptions {
  readonly type?: "png" | "jpeg";
  readonly quality?: number;
  readonly omitBackground?: boolean;
  readonly animations?: "allow" | "disabled";
  readonly caret?: "hide" | "initial";
  readonly scale?: "css" | "device";
  /** Covered with a box; Secrets are masked by the engine anyway (ADR 0014). */
  readonly mask?: readonly Locator[];
  readonly maskColor?: string;
  /**
   * Also written to this path on the program's machine, whole or not at all (ADR 0116): a path it
   * cannot write throws `PathNotWritableError`, full or failing storage `FileWriteFailedError`, and
   * the call then returns no bytes.
   */
  readonly path?: string;
}

export interface PageScreenshotOptions extends ScreenshotOptions {
  readonly fullPage?: boolean;
  readonly clip?: BoundingBox;
}

/**
 * A lazy description of elements, built without I/O and resolved again at each use. A cheap value:
 * composing one never touches the page.
 */
export interface Locator extends Locates, AIVerbs, AriaSnapshots, RawCall {
  filter(options: FilterOptions): Locator;
  and(locator: Locator): Locator;
  or(locator: Locator): Locator;
  /** Opts out of strictness: the first match. */
  first(): Locator;
  last(): Locator;
  nth(index: number): Locator;
  /** The content of the `iframe` this locator matches. */
  contentFrame(): FrameLocator;
  /** One locator per element matching now. Answers at once, with no timeout. */
  all(): Promise<Locator[]>;

  // Gestures: guarded, and run one at a time on the page.
  /** Waits until the target is unique, visible, stable, enabled and receiving the click, then clicks; a click that lands elsewhere is cancelled and replayed. */
  click(options?: ClickOptions): Promise<void>;
  dblclick(options?: ClickOptions): Promise<void>;
  tap(options?: PointerOptions): Promise<void>;
  hover(options?: PointerOptions): Promise<void>;
  check(options?: PointerOptions): Promise<void>;
  uncheck(options?: PointerOptions): Promise<void>;
  setChecked(checked: boolean, options?: PointerOptions): Promise<void>;
  dragTo(target: Locator, options?: ActionOptions & { readonly sourcePosition?: Position; readonly targetPosition?: Position }): Promise<void>;
  /**
   * Clears the field and inserts the text as typed text arrives, so controlled inputs see it. On a
   * `<label>`, acts on its control. A target that cannot hold text throws `TargetUnsupportedError` at
   * once; a read-only one is waited for.
   * A Secret placeholder is typed by the engine, checked against the domain of the field's frame
   * (ADR 0092); reads of the field then show `%name%`.
   */
  fill(value: string | SecretPlaceholder, options?: ActionOptions): Promise<void>;
  clear(options?: ActionOptions): Promise<void>;
  /** A key or a combination, such as `Enter` or `ControlOrMeta+A`. */
  press(key: string, options?: KeyOptions): Promise<void>;
  /** Types key by key, for fields that react to each keystroke; a Secret placeholder as `fill` types it. */
  pressSequentially(text: string | SecretPlaceholder, options?: KeyOptions): Promise<void>;
  /** On a `<select>`, or the control a `<label>` points to. Returns the values selected. */
  selectOption(values: SelectOptionValue | readonly SelectOptionValue[], options?: ActionOptions): Promise<string[]>;
  selectText(options?: ActionOptions): Promise<void>;
  /** On a file input, or the one a `<label>` points to. */
  setInputFiles(files: InputFiles, options?: ActionOptions): Promise<void>;
  focus(options?: CallOptions): Promise<void>;
  blur(options?: CallOptions): Promise<void>;
  scrollIntoViewIfNeeded(options?: CallOptions): Promise<void>;

  // Reads: one round trip each, whatever the number of matches.
  /** Waits for one element, as an action does, under the action timeout. */
  textContent(options?: CallOptions): Promise<string>;
  innerText(options?: CallOptions): Promise<string>;
  innerHTML(options?: CallOptions): Promise<string>;
  inputValue(options?: CallOptions): Promise<string>;
  /** `null` when the attribute is absent: an answer, not a failure. */
  getAttribute(name: string, options?: CallOptions): Promise<string | null>;
  /** `null` when the element is not visible. */
  boundingBox(options?: CallOptions): Promise<BoundingBox | null>;
  isEnabled(options?: CallOptions): Promise<boolean>;
  isDisabled(options?: CallOptions): Promise<boolean>;
  isChecked(options?: CallOptions): Promise<boolean>;
  isEditable(options?: CallOptions): Promise<boolean>;
  /** Answers at once, with no timeout: no match is not visible; two matches throw `TargetNotUniqueError`. */
  isVisible(): Promise<boolean>;
  isHidden(): Promise<boolean>;
  /** Answers at once. To wait for a count, `waitForCount`. */
  count(): Promise<number>;
  allTextContents(): Promise<string[]>;
  allInnerTexts(): Promise<string[]>;
  screenshot(options?: ScreenshotOptions): Promise<Uint8Array>;

  // Page functions.
  evaluate<R extends PageResult = JSONValue, Arg extends JSONValue = JSONValue>(
    pageFunction: ElementFunction<Arg, R>,
    arg?: Arg,
    options?: EvaluateOptions,
  ): Promise<R>;
  evaluateAll<R extends PageResult = JSONValue, Arg extends JSONValue = JSONValue>(
    pageFunction: ElementsFunction<Arg, R>,
    arg?: Arg,
    options?: EvaluateOptions,
  ): Promise<R>;

  // Waits (ADR 0080).
  /** Waits until the page function, given the element first, returns a truthy value; the locator is resolved again at each check. */
  waitForFunction<R extends PageResult = JSONValue, Arg extends JSONValue = JSONValue>(
    pageFunction: ElementFunction<Arg, R>,
    arg?: Arg,
    options?: CallOptions,
  ): Promise<R>;
  /** Playwright's `waitFor`: one element in the given state. */
  waitFor(options?: CallOptions & { readonly state?: "attached" | "detached" | "visible" | "hidden" }): Promise<void>;
  /** Waits until the number of matches is in range: a different behaviour, so a different name (S3). */
  waitForCount(count: Count, options?: CallOptions): Promise<number>;
}

// Requests and responses.

/** A newer engine's resource type is kept as received (G3). */
export type ResourceType =
  | "document"
  | "stylesheet"
  | "image"
  | "media"
  | "font"
  | "script"
  | "texttrack"
  | "xhr"
  | "fetch"
  | "eventsource"
  | "websocket"
  | "manifest"
  | "other"
  | (string & {});

/** Header names lower-cased, as the browser sent or received them. */
export type Headers = Readonly<Record<string, string>>;

/** A request the page sent, under Playwright's accessors. */
export interface Request {
  url(): string;
  method(): string;
  headers(): Headers;
  postData(): string | null;
  resourceType(): ResourceType;
  isNavigationRequest(): boolean;
  redirectedFrom(): Request | null;
  /** The browser's network error for a failed request, such as `net::ERR_CONNECTION_RESET`; `null` otherwise. */
  failure(): string | null;
  /** Its response, once received; `null` when it failed. */
  response(options?: CallOptions): Promise<Response | null>;
}

/** A response, from the page, from `route.fetch()` or from `page.request`. */
export interface Response {
  url(): string;
  status(): number;
  statusText(): string;
  /**
   * `true` for a status from 200 to 299. It says nothing about the page itself: a login page that
   * answers 200 is `ok`, and the Page check judges pages (ADR 0006).
   */
  ok(): boolean;
  headers(): Headers;
  request(): Request;
  /**
   * `true` when a site's service worker answered the request itself, typically from its own cache,
   * so it reached no network and no route answered it (ADR 0121). Playwright's accessor.
   */
  fromServiceWorker(): boolean;
  /**
   * Read on demand. Never fails for a response from `waitForRequestFinished`, `route.fetch()` or
   * `page.request`, whose bodies the engine keeps; elsewhere a body the browser has dropped throws
   * `BodyUnavailableError`.
   */
  body(options?: CallOptions): Promise<Uint8Array>;
  /**
   * The body decoded with the response's own encoding, where Playwright always decodes UTF-8 (ADR
   * 0119): a navigation's document response with the encoding its document was decoded with, so it
   * agrees with `page.content()`; any other with its BOM, its `Content-Type` charset, the `<meta>`
   * prescan for HTML or the XML declaration for XML, then UTF-8. Bad bytes become U+FFFD; it never
   * throws on them.
   */
  text(options?: CallOptions): Promise<string>;
  /** UTF-8 whatever the charset (RFC 8259). A body that is not JSON throws `ResponseNotJSONError`. Narrow the value with the program's own schema. */
  json(options?: CallOptions): Promise<JSONValue>;
}

/** Changes a route passes on with the request. A placeholder in a header is checked against the request's URL (ADR 0092). */
export interface RequestOverrides {
  readonly url?: string;
  readonly method?: string;
  readonly headers?: HeadersInput;
  readonly postData?: string | Uint8Array;
}

export interface Fulfillment {
  readonly status?: number;
  readonly headers?: Headers;
  readonly contentType?: string;
  readonly body?: string | Uint8Array;
  readonly json?: JSONValue;
  /** A response from `route.fetch()` to start from. */
  readonly response?: Response;
}

/** The network error an aborted request reports to the page, under Playwright's names. */
export type AbortReason =
  | "aborted"
  | "accessdenied"
  | "addressunreachable"
  | "blockedbyclient"
  | "blockedbyresponse"
  | "connectionaborted"
  | "connectionclosed"
  | "connectionfailed"
  | "connectionrefused"
  | "connectionreset"
  | "internetdisconnected"
  | "namenotresolved"
  | "timedout"
  | "failed";

/**
 * An intercepted request waiting for the program's answer. The first route to settle it wins:
 * `continue`, `fulfill`, `abort`, or `fetch` then `fulfill`. `fallback` passes it, with its
 * overrides, to the route registered before this one, and from the page's first route to the
 * Session's last; a handler that ends without settling falls back too, and past the last route the
 * request goes to the network. Settling twice, or after the
 * handler ended, throws `RouteAlreadyHandledError`. What a route sends on still passes the run's
 * domain policy (ADR 0014) and Filter lists (ADR 0047).
 */
export interface Route {
  /** The request as the page sent it, before any override. */
  request(): Request;
  continue(overrides?: RequestOverrides): Promise<void>;
  fulfill(response: Fulfillment): Promise<void>;
  abort(reason?: AbortReason): Promise<void>;
  fallback(overrides?: RequestOverrides): Promise<void>;
  /** Sends the request through the engine and returns its response, body kept, for `fulfill`. */
  fetch(overrides?: RequestOverrides & CallOptions): Promise<Response>;
}

/**
 * Answers matching requests. Its return value is ignored. If it throws, its request is aborted,
 * never sent half-changed, and the route is removed; `PageHandlerFailedError` is thrown by a call in
 * flight that waited on that request, or else by the next call on the value it was registered on,
 * as ADR 0081's listeners do.
 */
export type RouteHandler = (route: Route) => void | Promise<void>;

export interface RequestOptions extends CallOptions {
  readonly method?: string;
  /** A Secret placeholder in a header or a JSON string is resolved by the engine, checked against the request's URL (ADR 0092). */
  readonly headers?: HeadersInput;
  readonly params?: Readonly<Record<string, string | number | boolean>>;
  /** A string or bytes as they are; a JSON value as JSON. */
  readonly data?: string | Uint8Array | JSONValueWithSecrets;
  readonly maxRedirects?: number;
}

/**
 * Playwright's request API on the page's Session, sent through the browser-fetch tier so the
 * Session's identity does not change (ADR 0020). Its responses keep their bodies. Whether a request
 * is an Effect is declared as ADR 0010 lets a program declare one.
 */
export interface PageRequest {
  fetch(url: string, options?: RequestOptions): Promise<Response>;
  get(url: string, options?: Omit<RequestOptions, "method">): Promise<Response>;
  post(url: string, options?: Omit<RequestOptions, "method">): Promise<Response>;
  put(url: string, options?: Omit<RequestOptions, "method">): Promise<Response>;
  patch(url: string, options?: Omit<RequestOptions, "method">): Promise<Response>;
  delete(url: string, options?: Omit<RequestOptions, "method">): Promise<Response>;
  head(url: string, options?: Omit<RequestOptions, "method">): Promise<Response>;
}

// Events and requests that need an answer.

export type ConsoleMessageType =
  | "log"
  | "debug"
  | "info"
  | "error"
  | "warning"
  | "dir"
  | "table"
  | "trace"
  | "assert"
  | "count"
  | "timeEnd"
  | (string & {});

export interface ConsoleMessage {
  type(): ConsoleMessageType;
  text(): string;
  location(): { readonly url: string; readonly lineNumber: number; readonly columnNumber: number };
}

/** An exception the page's own scripts did not catch (`pageerror`). Playwright's name; never `PageError`, which is a group of Goodfetch's errors. */
export interface WebError {
  readonly name: string;
  readonly message: string;
  readonly stack: string | null;
}

/** A download, kept by the engine with its Run. */
export interface Download {
  url(): string;
  suggestedFilename(): string;
  /** Waits for the download to end: the reason it failed, or `null`. An answer, not a failure. */
  failure(options?: CallOptions): Promise<string | null>;
  /**
   * Streams the file to this path on the program's machine. A failed download throws
   * `DownloadFailedError`; a failed write `PathNotWritableError` or `FileWriteFailedError`, leaving
   * the path as it was (ADR 0116).
   */
  saveAs(path: string, options?: CallOptions): Promise<void>;
  cancel(options?: CallOptions): Promise<void>;
}

/** A file chooser the page opened. Answered with `setFiles`, a gesture. */
export interface FileChooser {
  isMultiple(): boolean;
  setFiles(files: InputFiles, options?: CallOptions): Promise<void>;
}

export interface WebSocketFrame {
  readonly payload: string | Uint8Array;
}

export interface WebSocketEventMap {
  readonly framesent: WebSocketFrame;
  readonly framereceived: WebSocketFrame;
  readonly close: void;
}

export interface WebSocket {
  url(): string;
  isClosed(): boolean;
  on<K extends keyof WebSocketEventMap>(event: K, listener: Listener<WebSocketEventMap[K]>): Subscription;
}

/** The page's observation events and what each carries, under Playwright's names (S3). */
export interface PageEventMap {
  readonly console: ConsoleMessage;
  readonly pageerror: WebError;
  readonly request: Request;
  /** The request finished, its body read. */
  readonly requestfinished: Request;
  readonly requestfailed: Request;
  readonly response: Response;
  readonly popup: Page;
  readonly download: Download;
  readonly filechooser: FileChooser;
  readonly websocket: WebSocket;
  readonly load: void;
  readonly domcontentloaded: void;
  readonly close: void;
  /** The renderer crashed; calls fail with `PageCrashedError`. */
  readonly crash: void;
}

/** A JavaScript dialog waiting for the program's answer. */
export interface Dialog {
  type(): "alert" | "confirm" | "prompt" | "beforeunload";
  message(): string;
  defaultValue(): string;
  accept(promptText?: string): Promise<void>;
  dismiss(): Promise<void>;
}

/**
 * Answers dialogs under the routes' rules: later handlers first, the first to accept or dismiss wins,
 * one that does neither falls through, and past the last one, or with none, the dialog gets
 * Playwright's default: an `alert`, `confirm` or `prompt` is dismissed, since accepting a `confirm()`
 * may confirm an Effect (ADR 0014), and a `beforeunload` dialog is accepted. A handler that throws:
 * the dialog gets its default, and `PageHandlerFailedError` is thrown by the gesture the dialog
 * blocked, or else by the next call on the page.
 */
export type DialogHandler = (dialog: Dialog) => void | Promise<void>;

export interface LocatorHandlerOptions {
  /** After the handler, do not wait for the locator to be hidden. */
  readonly noWaitAfter?: boolean;
  /** Removed after this many calls. */
  readonly times?: number;
}

// Input with no target: nothing to guard, so marked unguarded in Provenance.

export interface Keyboard {
  press(key: string, options?: CallOptions & { readonly delay?: number }): Promise<void>;
  down(key: string, options?: CallOptions): Promise<void>;
  up(key: string, options?: CallOptions): Promise<void>;
  type(text: string, options?: CallOptions & { readonly delay?: number }): Promise<void>;
  insertText(text: string, options?: CallOptions): Promise<void>;
}

export interface MouseButtonOptions extends CallOptions {
  readonly button?: "left" | "right" | "middle";
  readonly clickCount?: number;
}

export interface Mouse {
  move(x: number, y: number, options?: CallOptions & { readonly steps?: number }): Promise<void>;
  down(options?: MouseButtonOptions): Promise<void>;
  up(options?: MouseButtonOptions): Promise<void>;
  click(x: number, y: number, options?: MouseButtonOptions & { readonly delay?: number }): Promise<void>;
  dblclick(x: number, y: number, options?: MouseButtonOptions & { readonly delay?: number }): Promise<void>;
  wheel(deltaX: number, deltaY: number, options?: CallOptions): Promise<void>;
}

export interface Touchscreen {
  tap(x: number, y: number, options?: CallOptions): Promise<void>;
}

// The way down.

/**
 * Raw Chrome DevTools Protocol for one page, over the program's own connection. It is a raw CDP
 * attach under ADR 0017: the Session's traffic goes through the egress proxy (on a Borrowed browser,
 * ADR 0056's interception instead), no Secret is typed, its
 * results are marked unguarded and the attach is in the Audit log. It reaches only its page's target:
 * commands that close or attach other targets, and anything that would close a Borrowed browser,
 * throw `CDPCommandRefusedError`. Semver covers this shape, never the protocol, which is Chromium's.
 */
export interface CDPSession extends Closeable {
  send(method: string, params?: JSONObject, options?: CallOptions): Promise<JSONValue>;
  on(event: string, listener: Listener<JSONValue>): Subscription;
}

export interface NavigationOptions extends CallOptions {
  readonly waitUntil?: LoadState;
  readonly referer?: string;
}

export interface PDFOptions extends CallOptions {
  readonly format?: string;
  readonly landscape?: boolean;
  readonly printBackground?: boolean;
  readonly scale?: number;
  readonly pageRanges?: string;
  /**
   * Also written to this path on the program's machine, whole or not at all (ADR 0116): a path it
   * cannot write throws `PathNotWritableError`, full or failing storage `FileWriteFailedError`, and
   * the call then returns no bytes.
   */
  readonly path?: string;
}

/** Options of closing a page. */
export interface PageCloseOptions extends CallOptions {
  /** Runs the page's `beforeunload` handlers, which `close()` otherwise skips; a navigation can still raise one. */
  readonly runBeforeUnload?: boolean;
}

/** The page API beside the waits (ADR 0080) and events (ADR 0081). */
export interface PageAPI extends Locates {
  // Navigations are gestures; their timeout is the navigation phase's.
  /** A 404 or a 500 is a response, a value; `null` for a navigation within the same document. */
  goto(url: string, options?: NavigationOptions): Promise<Response | null>;
  reload(options?: NavigationOptions): Promise<Response | null>;
  goBack(options?: NavigationOptions): Promise<Response | null>;
  goForward(options?: NavigationOptions): Promise<Response | null>;
  setContent(html: string, options?: CallOptions & { readonly waitUntil?: LoadState }): Promise<void>;
  /** Read without a round trip, kept current by the engine's navigation events. */
  url(): string;
  title(options?: CallOptions): Promise<string>;
  content(options?: CallOptions): Promise<string>;
  isClosed(): boolean;
  /** The Session this page belongs to: Playwright's `page.context()`. */
  session(): Session;

  readonly keyboard: Keyboard;
  readonly mouse: Mouse;
  readonly touchscreen: Touchscreen;
  readonly request: PageRequest;

  evaluate<R extends PageResult = JSONValue, Arg extends JSONValue = JSONValue>(
    pageFunction: PageFunction<Arg, R>,
    arg?: Arg,
    options?: EvaluateOptions,
  ): Promise<R>;
  /** Waits until the page function returns a truthy value, and returns it; `polling` is `raf` or milliseconds. */
  waitForFunction<R extends PageResult = JSONValue, Arg extends JSONValue = JSONValue>(
    pageFunction: PageFunction<Arg, R>,
    arg?: Arg,
    options?: CallOptions & { readonly polling?: "raf" | number },
  ): Promise<R>;
  /** Runs before the page's own scripts in every new document, marked in Provenance; registered when called. */
  addInitScript<Arg extends JSONValue = JSONValue>(script: PageFunction<Arg, PageResult>, arg?: Arg): Subscription;

  screenshot(options?: PageScreenshotOptions): Promise<Uint8Array>;
  /** Chromium only. */
  pdf(options?: PDFOptions): Promise<Uint8Array>;

  /**
   * Answers requests matching `url`, registered when called. A glob or a regular expression matches in
   * the engine; a predicate runs here and, like a route on every URL, has every request wait for the program (ADR 0081).
   * Page routes run before the Session's.
   */
  route(url: URLMatcher, handler: RouteHandler): Subscription;
  /**
   * Before each attempt of a guarded action, when `locator` is visible, the guard calls the handler
   * in the program, then waits for the locator to be hidden. Handlers run one at a time, their own
   * actions trigger none, and their time counts toward the action's timeout; one that throws fails
   * the action it interrupted with `PageHandlerFailedError` and stays registered. While a handler
   * runs, the interrupted action gives up the page's turn, so the handler's own gestures run.
   */
  addLocatorHandler(
    locator: Locator,
    handler: (locator: Locator) => void | Promise<void>,
    options?: LocatorHandlerOptions,
  ): Subscription;

  /** Chromium only: elsewhere it throws `CapabilityUnavailableError`. */
  newCDPSession(options?: CallOptions): Promise<CDPSession>;
}

// The page API's own Error codes (ADR 0079's naming rule).

/** A selector that does not parse, or uses a Playwright-only engine, `>>` or a Playwright CSS extension (`:has-text()`, `:visible`, …); `fix` names the replacing method. */
export declare class SelectorInvalidError extends ProgramError {
  readonly code: "selector_invalid";
  readonly details: { readonly selector: string; readonly construct: string };
}

/** The target is of the wrong kind for the action, such as `fill` on a `<div>`. Thrown at once: an element's kind does not change. */
export declare class TargetUnsupportedError extends PageError {
  readonly code: "target_unsupported";
  readonly details: { readonly action: string; readonly expected: readonly string[]; readonly found: string };
}

/** The page function threw. A `ReferenceError`'s fix names the argument: a page function sees only its argument. */
export declare class PageFunctionFailedError extends ProgramError {
  readonly code: "page_function_failed";
  readonly details: { readonly name: string; readonly message: string; readonly stack: string | null };
}

/** A value crossing to or from the page is not JSON: a non-finite number, a node, a function, a `Map`, a cycle. */
export declare class PageValueInvalidError extends ProgramError {
  readonly code: "page_value_invalid";
  readonly details: { readonly direction: "argument" | "result"; readonly path: string; readonly kind: string };
}

/** A route, dialog or locator handler threw; `cause` holds its exception in this process. */
export declare class PageHandlerFailedError extends ProgramError {
  readonly code: "page_handler_failed";
  readonly details: { readonly handler: "route" | "dialog" | "locator"; readonly handled: string };
}

export declare class RouteAlreadyHandledError extends ProgramError {
  readonly code: "route_already_handled";
}

/** A navigation superseded by another before it committed. */
export declare class NavigationInterruptedError extends PageError {
  readonly code: "navigation_interrupted";
  readonly details: { readonly by: string };
}

/**
 * On a page other code also drives (a page borrowed with `run.borrow()`, a Lent page while a library is
 * attached, a page with an open CDP session), an AI verb's page committed a navigation to a new
 * document that the verb did not start, between its first read of the page and its return (ADR 0114).
 * Nothing was written to the Plan, and an `act` sent no input. Retryable: await each Collie call
 * before the program's own driver acts on the page; the page may also have navigated by itself.
 */
export declare class PageNavigatedDuringCallError extends PageError {
  readonly code: "page_navigated_during_call";
  readonly details: { readonly url: string; readonly navigatedTo: string };
}

export declare class ResponseNotJSONError extends PageError {
  readonly code: "response_not_json";
}

/** The site never answered: the browser's network error, such as `net::ERR_NAME_NOT_RESOLVED`. Retryable. */
export declare class NavigationFailedError extends PageError {
  readonly code: "navigation_failed";
  readonly details: { readonly netError: string };
}

export declare class PageCrashedError extends UnavailableError {
  readonly code: "page_crashed";
}

/** The browser no longer holds the body; `fix` names `waitForRequestFinished`. */
export declare class BodyUnavailableError extends UnavailableError {
  readonly code: "body_unavailable";
}

export declare class DownloadFailedError extends UnavailableError {
  readonly code: "download_failed";
  readonly details: { readonly reason: string };
}

export declare class CDPCommandRefusedError extends RefusedError {
  readonly code: "cdp_command_refused";
  readonly details: { readonly method: string };
}
