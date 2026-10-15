// The testing kit's shared part (ADRs 0069 and 0105, decided in #482), `@goodfetch/collie-client/testing`:
// what both Products' kits share. Each Product ships its own kit, re-exporting this one and adding its
// `TestEngine` (ADRs 0090, 0095): `@goodfetch/collie/testing` and `@goodfetch/foxhound/testing`, whose
// engine adds `runHandler` and `runHook`, since crawl handlers and Hooks are Foxhound's (ADR 0084).
// It fits any test runner: plain values closed with `await using`, failures thrown as
// `GoodfetchError`s, and assertions left to the runner's own `expect` or `assert`.
//
// - `TestEngine`: ADR 0069's test engine, a real engine set up for one test. It reaches no site unless
//   a Run records: every Run that names no Response recording replays an empty one.
// - `scriptedModel`: ADR 0069's Scripted model, a Model provider the test registers.
// - `fakeError`: any `GoodfetchError`, for a test of the program's own error handling. Result values
//   are plain interfaces, so a test writes them as object literals.

import type { Connection, GoodfetchError, Logger, ModelProviderHandler, ModelRequest, ModelResponse, ProgramError, Settings } from "@goodfetch/collie-client";

/** Options of starting a test engine. */
export interface TestEngineOptions<S = Settings> {
  /**
   * Settings for its connection and everything beneath it (ADR 0083). Every host its `resolve`
   * setting (ADR 0059) maps to loopback, such as a test's own server, is allowlisted (ADR 0017) and
   * reached live even while a Run replays.
   */
  readonly settings?: S;
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
export interface TestEngine<C extends Connection> extends AsyncDisposable {
  /** A connection to this engine, its Product's connection type: everything a program does works on it. */
  readonly connection: C;
  /**
   * Stands for a person's answer about an uncertain Effect of a Run on this engine (ADR 0029): the
   * key's next declaration returns `reconciled` after `happened`, and performs the body again after
   * `did_not_happen` (ADR 0106). Only the testing kit has it.
   */
  reconcile(runId: string, key: string, answer: "happened" | "did_not_happen"): Promise<void>;
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
/** The test engine closed with replies of a Scripted model unused: the code under test called the model less than the test expected. */
export declare class ScriptedModelUnusedError extends ProgramError {
  readonly code: "scripted_model_unused";
  readonly details: { readonly provider: string; readonly remaining: number };
}
