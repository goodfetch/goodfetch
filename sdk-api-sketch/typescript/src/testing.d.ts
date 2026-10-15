// The testing kit, `goodfetch/testing` (ADRs 0069 and 0105, decided in #482): a subpath export,
// public and under semver. Each Product ships its own (ADRs 0090, 0095); crawl handlers and Hooks are
// Foxhound's (ADR 0084). It fits any test runner: plain values closed with `await using`, failures
// thrown as `GoodfetchError`s, and assertions left to the runner's own `expect` or `assert`.
//
// - `TestEngine`: ADR 0069's test engine, a real engine set up for one test. It reaches no site unless
//   a Run records: every Run that names no Response recording replays an empty one.
// - `runHandler`: one crawl handler on one saved response, reporting how its attempt ended and what it
//   staged, without a Frontier, Hooks or a commit.
// - `runHook`: one of the crawl's Hooks, by name, the same way.
// - `scriptedModel`: ADR 0069's Scripted model, a Model provider the test registers.
// - `fakeError`: any `GoodfetchError`, for a test of the program's own error handling. Result values
//   are plain interfaces, so a test writes them as object literals.

import type { Crawl, FetchTier, ReportedFetchTier, Enqueued } from "./crawl.js";
import type { Dataset, JsonValue, Label, PayloadInput } from "./data.js";
import type { Goodfetch, Logger } from "./lifecycle.js";
import type { ModelProviderHandler, ModelRequest, ModelResponse, ModelUsage } from "./models.js";
import type { StoredBytes } from "./output.js";
import type { Headers } from "./page.js";
import type { BlockCause, GoodfetchError, ProgramError, SkipReason } from "./results.js";
import type { Settings } from "./settings.js";

/** Options of starting a test engine. */
export interface TestEngineOptions {
  /**
   * Settings for its connection and everything beneath it (ADR 0083). Every host its `resolve`
   * setting (ADR 0059) maps to loopback, such as a test's own server, is allowlisted (ADR 0017) and
   * reached live even while a Run replays.
   */
  readonly settings?: Settings;
  /**
   * The environment map, `connect()`'s own option (ADRs 0062, 0083): the process's own when left out.
   * Only `GOODFETCH_HOME` (to read its browser cache, never to write), `GOODFETCH_BROWSERS_PATH` and
   * this Product's engine-path variable are read from it; the engine's own environment stays cleared.
   */
  readonly env?: Readonly<Record<string, string>>;
  /** The project configuration file. Left out: none, as the home and the environment are the test's own. */
  readonly project?: string;
  /** Model providers the test answers itself, such as a Scripted model, named by `ai.model` as `"<name>/<any>"` (ADR 0101). */
  readonly modelProviders?: Readonly<Record<string, ModelProviderHandler>>;
  readonly logger?: Logger;
}

/** The document a handler runs on, given as `route.fulfill` gives a response, or read from a saved file. */
export interface SavedResponse {
  /** Default 200. */
  readonly status?: number;
  readonly headers?: Headers;
  /** Default: from `path`'s extension, else `text/html`. */
  readonly contentType?: string;
  readonly body?: string | Uint8Array;
  /** A file on the program's machine, relative to the working directory, such as a page saved from a browser. */
  readonly path?: string;
}

/** What `runHandler` runs a handler on: one crawl request of the Label. */
export type HandlerInput<L extends Label<unknown, unknown>> = {
  /** The request's URL, the scope's seed, and the document's URL unless the response redirects. */
  readonly url: string | URL;
  /** The document. Left out: the response `recording` holds for `url`. */
  readonly response?: SavedResponse;
  /** A Response recording replayed for everything else the handler reaches. Left out: none, so it reaches nothing. */
  readonly recording?: string;
  /**
   * `http` by default, or the tier the Label's `fetch.tier` forces; another than a forced one throws
   * `SettingInvalidError`. No browser starts on a non-browser tier: a call only a browser serves ends
   * the attempt `escalated` under `fetch.tier: auto`, and `failed` with `browser_required` under a
   * forced tier (ADR 0093). `browser` renders the document in the browser, its `Date` starting at the
   * document's `Date` header.
   */
  readonly tier?: FetchTier;
  /** Default 1, for a handler that acts on its attempt number. */
  readonly attempt?: number;
  /** Default 0. */
  readonly depth?: number;
} & ([PayloadInput<L>] extends [undefined] ? { readonly payload?: undefined } : { readonly payload: PayloadInput<L> });

/** How an attempt ended. */
export type AttemptEnd =
  /** The handler returned: what it staged would commit. */
  | { readonly kind: "returned" }
  /** `ctx.skip(message)` (reason `handler`), or a handler's navigation that Site preferences refused and it let escape. */
  | { readonly kind: "skipped"; readonly reason: SkipReason; readonly message: string | null }
  /** `ctx.retry(message)`. */
  | { readonly kind: "retry_requested"; readonly message: string }
  /** A call the tier cannot serve; in a crawl, the request would run again in the browser. `call` is that call, rendered as the program wrote it (ADR 0079). */
  | { readonly kind: "escalated"; readonly call: string }
  /** The document's status is a Block cause (ADRs 0021, 0093): the handler did not run. */
  | { readonly kind: "blocked"; readonly cause: BlockCause }
  /**
   * The handler threw: its own exception as `handler_failed`, or the code a `GoodfetchError` keeps; or,
   * before it ran, the document's status failed it (`response_not_ok`, `response_unavailable`).
   */
  | { readonly kind: "failed"; readonly error: GoodfetchError }
  /** A kind this SDK does not know, as received (G3). */
  | { readonly kind: "unknown"; readonly raw: string };

/** An enqueue or a seed the attempt staged, with the answer it got. */
export interface StagedRequest {
  readonly url: string;
  readonly label: string;
  readonly payload: JsonValue;
  readonly answer: Enqueued;
}

/** A Key-value store write the attempt staged; `null` for a delete. */
export interface StagedWrite {
  readonly key: string;
  readonly value: JsonValue | StoredBytes | null;
}

/**
 * One attempt of a handler: how it ended and everything it staged, kept even when the attempt ended
 * early, where a crawl would discard it (ADR 0003). Nothing is committed.
 */
export interface AttemptResult {
  readonly ended: AttemptEnd;
  readonly tier: ReportedFetchTier;
  /** The Records pushed to a Dataset, as `run.records()` would read them back. */
  records<O>(dataset: Dataset<unknown, O>): readonly O[];
  readonly enqueued: readonly StagedRequest[];
  /** `ctx.run.addSeed()` calls. */
  readonly seeds: readonly StagedRequest[];
  readonly keyValueWrites: readonly StagedWrite[];
  /** Model calls made by AI verbs in the handler (ADR 0101). */
  readonly usage: ModelUsage;
}

/**
 * What `runHook` builds the Hook's stage context from (ADR 0104): the request every context carries,
 * and what its stage adds: before a request, its method, body, headers, tier and previous error; after
 * one, the `response`; for the page stages, Collie's page on that document; before a Record's commit,
 * the `dataset` and `record`.
 */
export interface HookInput {
  readonly label: Label<unknown, unknown>;
  readonly url: string | URL;
  readonly payload?: JsonValue;
  /** Default `GET`. */
  readonly method?: string;
  /** Default none. */
  readonly body?: string | Uint8Array;
  /** The headers as built so far, the enqueue's own included. Default none. */
  readonly headers?: Headers;
  /** The tier about to fetch: `http` by default, or the one the Label forces. */
  readonly tier?: FetchTier;
  /** The error of the attempt before, for a Hook's retry branch. Default `null`, a first attempt. */
  readonly previousError?: GoodfetchError | null;
  /** Default 1. */
  readonly attempt?: number;
  /** Default 0. */
  readonly depth?: number;
  readonly response?: SavedResponse;
  readonly recording?: string;
  readonly dataset?: Dataset<unknown, unknown>;
  readonly record?: JsonValue;
}

/** What a Hook did: its answer, or the call that ended the work. */
export type HookResult =
  | { readonly kind: "answered"; readonly value: JsonValue | null }
  | { readonly kind: "skipped" | "retry_requested" | "dropped" | "rejected"; readonly message: string }
  /** `block(cause)` after a response (ADR 0104). */
  | { readonly kind: "blocked"; readonly cause: BlockCause }
  | { readonly kind: "failed"; readonly error: GoodfetchError }
  | { readonly kind: "unknown"; readonly raw: string };

/**
 * A real engine for one test: a temporary Goodfetch home removed when it closes, a cleared
 * environment, no project file unless named, `offline` (ADR 0043), short durations, and no Model
 * provider unless the test registers or configures one, so a call that needs a model with none
 * throws `ModelNotConfiguredError`. Every Run started from `connection` that names no Response
 * recording replays an empty one: a request to a site throws `ResponseNotRecordedError`, naming it.
 * Browsers come from the browser cache of the caller's Goodfetch home, read and never written, or
 * `GOODFETCH_BROWSERS_PATH` (ADRs 0024, 0059, 0105). An Attached
 * engine here; the linked engine in Rust (ADR 0069).
 */
export declare class TestEngine implements AsyncDisposable {
  private constructor();
  static start(options?: TestEngineOptions): Promise<TestEngine>;
  /** A connection to this engine, `Goodfetch.connect()`'s type: everything a program does works on it. */
  readonly connection: Goodfetch;
  /**
   * Runs the Label's handler, the one the crawl attached, on one request, with the Label's settings,
   * in a Run of its own: one attempt, no retry. Nothing else of the crawl runs: no Frontier, no Hooks,
   * no commit. The document is not fetched, so neither robots.txt nor Politeness applies to it, but
   * ADR 0093's status rules do, before the handler runs. Throws
   * when it cannot run the handler (`ResponseNotRecordedError` for the document, `PayloadInvalidError`,
   * `LabelUnknownError`, `LabelUnhandledError`, `SettingInvalidError` for the tier); once the handler
   * ran, how its attempt ended is a value.
   */
  runHandler<L extends Label<unknown, unknown>>(crawl: Crawl, label: L, input: HandlerInput<L>): Promise<AttemptResult>;
  /**
   * Runs one of the crawl's Hooks alone, by the name Hooks carry (#481), its stage's context built from
   * `input` as `runHandler` builds a handler's. A name the crawl does not hold throws `HookUnknownError`,
   * naming the closest.
   */
  runHook(crawl: Crawl, name: string, input: HookInput): Promise<HookResult>;
  /** Closes the connection and the engine and removes the home. Throws `ScriptedModelUnusedError` if a Scripted model registered here has replies left. */
  close(): Promise<void>;
  [Symbol.asyncDispose](): Promise<void>;
}

/** One reply of a Scripted model: a response, or an error the provider would give. A retryable error is retried as a provider's (ADR 0101), the retry taking the next reply. */
export type ScriptedReply = ModelResponse | GoodfetchError;

/** ADR 0069's Scripted model: answers call n with reply n, records every request, and reports Usage from its replies. */
export interface ScriptedModel extends ModelProviderHandler {
  /** Every request it answered, in order, for a test to check what the engine asked. */
  readonly requests: readonly ModelRequest[];
  /** Replies not used yet. */
  readonly remaining: number;
}

/** A Scripted model; register it in `TestEngineOptions.modelProviders`. A call beyond its replies throws `ScriptedModelExhaustedError`. */
export declare function scriptedModel(replies: readonly ScriptedReply[]): ScriptedModel;

/** Builds the error a call would throw: its class, group, retryable flag and details, for a test of the program's own error handling. */
export declare function fakeError<E extends GoodfetchError>(
  errorClass: abstract new (...args: never[]) => E,
  init?: { readonly message?: string; readonly details?: E["details"] },
): E;

/** A call went beyond the Scripted model's replies: the AI call that made it fails with this code (ADR 0101). */
export declare class ScriptedModelExhaustedError extends ProgramError {
  readonly code: "scripted_model_exhausted";
  readonly details: { readonly provider: string; readonly calls: number };
}
/** `runHook` named a Hook the crawl does not hold. */
export declare class HookUnknownError extends ProgramError {
  readonly code: "hook_unknown";
  readonly details: { readonly name: string; readonly closest: string | null };
}
/** The test engine closed with replies of a Scripted model unused: the code under test called the model less than the test expected. */
export declare class ScriptedModelUnusedError extends ProgramError {
  readonly code: "scripted_model_unused";
  readonly details: { readonly provider: string; readonly remaining: number };
}
