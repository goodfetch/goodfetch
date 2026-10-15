// Foxhound's testing kit, `@goodfetch/foxhound/testing` (ADRs 0069, 0105): the shared kit of
// `@goodfetch/collie-client/testing`, and a test engine whose connection is `Foxhound`, with
// `runHandler` (one crawl handler on one saved response, reporting how its attempt ended and what it
// staged, without a Frontier, Hooks or a commit) and `runHook` (one of the crawl's Hooks, by name,
// the same way).

export * from "@goodfetch/collie-client/testing";

import type {
  BlockCause,
  GoodfetchError,
  Headers,
  JSONValue,
  ModelUsage,
  ProgramError,
  StoredBytes,
  UncertainWhy,
} from "@goodfetch/collie-client";
import type { TestEngine as SharedTestEngine, TestEngineOptions } from "@goodfetch/collie-client/testing";
import type { Crawl, Enqueued, FetchTier, ReportedFetchTier } from "./crawl.js";
import type { Dataset, Label, PayloadInput } from "./data.js";
import type { Foxhound } from "./connection.js";
import type { SkipReason } from "./outcomes.js";
import type { Settings } from "./settings.js";

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
  /**
   * The Effect journal's prior answers, by Effect key (ADR 0106): a declaration of such a key is
   * answered as the journal would answer it, without running its body.
   */
  readonly effects?: Readonly<Record<string, PriorEffect>>;
} & ([PayloadInput<L>] extends [undefined] ? { readonly payload?: undefined } : { readonly payload: PayloadInput<L> });

/** A prior answer the Effect journal holds for a key in a test (ADR 0106). */
export type PriorEffect =
  | { readonly source: "journal"; readonly output: JSONValue }
  | { readonly source: "reconciled" }
  | { readonly source: "uncertain"; readonly why: UncertainWhy };

/** One declaration an attempt made, and how it was answered. */
export interface DeclaredEffect {
  readonly key: string;
  readonly answer: "performed" | "journal" | "reconciled" | "uncertain" | "not_performed" | (string & {});
  /** Whether its body sent anything; a command a Response recording answered counts as sent. */
  readonly sent: boolean;
}

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
  readonly payload: JSONValue;
  readonly answer: Enqueued;
}

/** A Key-value store write the attempt staged; `null` for a delete. */
export interface StagedWrite {
  readonly key: string;
  readonly value: JSONValue | StoredBytes | null;
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
  /** The Effects it declared, in order (ADR 0106). */
  readonly effects: readonly DeclaredEffect[];
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
  readonly payload?: JSONValue;
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
  readonly record?: JSONValue;
}

/** What a Hook did: its answer, or the call that ended the work. */
export type HookResult =
  | { readonly kind: "answered"; readonly value: JSONValue | null }
  | { readonly kind: "skipped" | "retry_requested" | "dropped" | "rejected"; readonly message: string }
  /** `block(cause)` after a response (ADR 0104). */
  | { readonly kind: "blocked"; readonly cause: BlockCause }
  | { readonly kind: "failed"; readonly error: GoodfetchError }
  | { readonly kind: "unknown"; readonly raw: string };

/** Foxhound's test engine: the shared one, with a `Foxhound` connection and a crawl's handlers and Hooks run alone. */
export declare class TestEngine implements SharedTestEngine<Foxhound> {
  private constructor();
  static start(options?: TestEngineOptions<Settings>): Promise<TestEngine>;
  readonly connection: Foxhound;
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
   * Runs one of the crawl's Hooks alone, by the name Hooks carry (ADR 0104), its stage's context built from
   * `input` as `runHandler` builds a handler's. A name the crawl does not hold throws `HookUnknownError`,
   * naming the closest.
   */
  runHook(crawl: Crawl, name: string, input: HookInput): Promise<HookResult>;
  reconcile(runId: string, key: string, answer: "happened" | "did_not_happen"): Promise<void>;
  close(): Promise<void>;
  [Symbol.asyncDispose](): Promise<void>;
}

/** `runHook` named a Hook the crawl does not hold. */
export declare class HookUnknownError extends ProgramError {
  readonly code: "hook_unknown";
  readonly details: { readonly name: string; readonly closest: string | null };
}
