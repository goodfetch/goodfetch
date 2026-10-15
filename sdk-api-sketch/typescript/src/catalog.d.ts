// Mapping the catalog to each language and surface (ADR 0099, decided in #471).
// - Every name is the catalog's canonical snake_case words, spelled by one rule per language:
//   camelCase members and PascalCase types, an initialism from the closed list in capitals unless
//   it opens a member's name (`waitForURL`, `baseURL`, `CDPSession`, `url()`). `id` is a word (`runId`).
// - Reserved words need no escape in TypeScript (`route.continue()`); `then` and `constructor` are
//   refused as names, since a member named `then` makes a value thenable.
// - A read the SDK answers itself on a handle is a method, as all of Playwright's are (`page.url()`,
//   `response.ok()`, `connection.engine()`); a sub-object reached without I/O is a `readonly` property
//   (`page.keyboard`, `connection.identities`); plain data has `readonly` properties.
// - The sketch's other initialism names (`JsonValue`, `JsonObject`, `JsonSchema`, `UrlMatcher`,
//   `PageApi`, `AiWorkResult`, `ResponseNotJsonError`) become `JSONValue`, `JSONObject`, `JSONSchema`,
//   `URLMatcher`, `PageAPI`, `AIWorkResult` and `ResponseNotJSONError` when #483 rewrites the sketch, and
//   reads on handles still spelled as properties (`run.id`, `run.startedAs`) become methods.

import type { JsonObject, JsonValue } from "./data.js";
import type { Closeable, EngineInfo } from "./lifecycle.js";
import type { Identities } from "./sessions.js";
import type { ExplainsSettings } from "./settings.js";
import type { CallOptions } from "./time.js";

/**
 * The raw catalog call: the Client layer's way down (G8). It calls an operation this SDK does not
 * generate, such as one a newer engine minor added, or one of Foxhound's from a Collie program on a
 * Foxhound engine. `params` and the result are the wire's JSON, in snake_case and never camelCased.
 *
 * It keeps the main path's protections: a closed value fails with `ClosedError`, the call's timeout
 * and `signal` apply, failures raise the same error classes (an unknown code arrives as its group's
 * class), the SDK never retries it, the engine validates `params` strictly and applies every policy,
 * and the Run trace records it, marked raw. It returns the result only, never events.
 */
export interface RawCall {
  /**
   * On a value, `operation` must be in the value's own namespace (`page.…` on a Page), and the value
   * fills the parameter of that name; any other operation fails with a `ProgramError` before it is
   * sent. On a connection, `params` names every value itself.
   */
  call(operation: string, params?: JsonObject, options?: CallOptions): Promise<JsonValue>;
}

/**
 * Any connection: what `Collie` and `Foxhound` share, declared once in `@goodfetch/collie-client`
 * (ADR 0095), so a library typed for it accepts either Product's connection. Collie's `start` and
 * `openRun` (ADR 0102) join it when #483 replaces the sketch's placeholder `Goodfetch`, which keeps
 * them on `gf.collie`, with `Collie` and `Foxhound`, whose connection is the Product.
 */
export interface Connection extends Closeable, ExplainsSettings, RawCall {
  /** The engine reached, fixed when the connection opened. */
  engine(): EngineInfo;
  /** The install's Saved identities (ADR 0092): a sub-object reached without I/O. */
  readonly identities: Identities;
}
