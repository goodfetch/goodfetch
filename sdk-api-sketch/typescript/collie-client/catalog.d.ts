// Mapping the catalog to each language and surface (ADR 0099, decided in #471).
// - Every name is the catalog's canonical snake_case words, spelled by one rule per language:
//   camelCase members and PascalCase types, an initialism from the closed list in capitals unless
//   it opens a member's name (`waitForURL`, `baseURL`, `CDPSession`, `url()`). `id` is a word (`runId`).
// - Reserved words need no escape in TypeScript (`route.continue()`); `then` and `constructor` are
//   refused as names, since a member named `then` makes a value thenable.
// - A read the SDK answers itself on a handle is a method, as all of Playwright's are (`page.url()`,
//   `response.ok()`, `connection.engine()`); a sub-object reached without I/O is a `readonly` property
//   (`page.keyboard`, `connection.identities`); plain data has `readonly` properties.
// - Every name in this sketch follows these rules (#483): `JSONValue`, `URLMatcher`, `PageAPI`,
//   `AIWorkResult`, `ResponseNotJSONError`, and reads on handles as methods (`run.id()`,
//   `run.startedAs()`, `crawlRun.seeds()`). The hand-written members are listed in the sketch's `manifest.md`.

import type { JSONObject, JSONValue } from "./data.js";
import type { Closeable, EngineInfo, Run } from "./lifecycle.js";
import type { RunView } from "./run.js";
import type { Identities } from "./sessions.js";
import type { ExplainsSettings } from "./settings.js";
import type { CallOptions, StartOptions } from "./time.js";

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
  call(operation: string, params?: JSONObject, options?: CallOptions): Promise<JSONValue>;
}

/**
 * Any connection: what `Collie` and `Foxhound` share, declared once in `@goodfetch/collie-client`
 * (ADRs 0095, 0099), so a library typed for it accepts either Product's connection. Each Product's
 * package holds its connection class, with its own `connect()`; a Foxhound connection carries every
 * member of a Collie connection under the same names.
 */
export interface Connection extends Closeable, ExplainsSettings, RawCall {
  /** The engine reached, fixed when the connection opened. */
  engine(): EngineInfo;
  /** The install's Saved identities (ADR 0092): a sub-object reached without I/O. */
  readonly identities: Identities;
  /**
   * Starts a Run and returns once it is started (ADR 0102). With an `id` that names an existing Run
   * with the same inputs: resumes it if interrupted (`startedAs()` is `"resumed"`), returns it as it
   * is if it finished (`"ended"`), or throws `RunInUseError` if it is live, `RunConflictError` for
   * other inputs, `LimitReachedError` if a used-up Limit stopped it, `RunStateUnavailableError` if its
   * state is gone.
   */
  start(options?: StartOptions): Promise<Run>;
  /** Opens a Run by its ID to read it, never to drive it: a detached Run, another program's, or one that ended. Throws `RunNotFoundError`. */
  openRun(id: string, options?: CallOptions): Promise<RunView>;
}
