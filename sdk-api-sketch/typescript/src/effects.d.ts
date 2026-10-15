// Effects a program declares in code (ADR 0106, decided in #500).
// - A program's own calls are Effects only when it declares them (ADR 0010): `run.effect(key, body)`
//   wraps a block of its code, and the value the body returns is kept in the Run's Effect journal.
// - In a crawl handler it is `ctx.run.effect(…)`: the key's namespace is the crawl request, so every
//   retry, escalation, sample comparison, lost lease and `retryFailed` reaches the same entry. Elsewhere
//   it is the Run, so a Collie program started again with its Run's ID finds what it already did.
// - The engine judges a body by what it sent for it while the body ran: a body that returned is done
//   and never runs again; one that ended before anything was sent was not performed; one that ended
//   after a send is uncertain (`effect_uncertain`) until a person reconciles it (ADR 0029), unless it
//   is repeatable. A body counts as sent from its start while the engine cannot see every command (a
//   raw CDP attach, a lent page, a browser shared with other code). Prepare the page before the
//   declaration: the body holds the action that commits the Effect and the reads of its result.
// - No separate check: a body that checks first, or sends `effect.idempotencyKey`, is repeatable.

import type { JsonValue } from "./data.js";
import type { CrawlRequestRef, ProgramError } from "./results.js";

/** The Effect a body performs, handed to the body. */
export interface Effect {
  /** The key it was declared with. */
  readonly key: string;
  /**
   * An opaque key, the same on every attempt and resume of this Effect, on any machine, and different
   * in every Run and every install. Send it as a site's `Idempotency-Key` header, or to the program's
   * own service, to make the body safe to repeat. It reveals neither the Run's ID nor the key.
   */
  readonly idempotencyKey: string;
}

/** Options of declaring an Effect. */
export interface EffectOptions {
  /**
   * The body is safe to run again: one that ended without returning after it sent something runs
   * again at the next declaration, instead of being reported uncertain. A done Effect never runs
   * again. Built-in: false (ADR 0010).
   */
  readonly repeatable?: boolean;
}

/**
 * What `run.effect` returns, narrowed by `source`:
 * - `performed`: the body ran now;
 * - `journal`: the Effect journal already held it from an earlier attempt or start, so the body did
 *   not run, and `output` is what it returned then;
 * - `reconciled`: a person answered "it happened" (ADR 0029), so the body did not run and no output
 *   is known;
 * - `unknown`: a source a newer engine sends, kept as received (ADR 0079).
 */
export type EffectResult<T> =
  | { readonly source: "performed" | "journal"; readonly output: T; readonly performedAt: Date }
  | { readonly source: "reconciled"; readonly reconciledBy: string; readonly reconciledAt: Date }
  | { readonly source: "unknown"; readonly raw: string };

/** The value a body returns: JSON, kept up to `effect.output.max_size`, or nothing. */
export type EffectOutput = JsonValue | void;

/** Declaring Effects: on every Run, and through a handler's `ctx.run`, bound to its attempt. */
export interface DeclaresEffects {
  /**
   * Performs the body as the Effect `key`, unless the Run's Effect journal already answers it.
   * The key, at most 255 code points, names one Effect in its namespace: the crawl request in a
   * handler or a Hook, the Run elsewhere; declaring it again while its body runs, or after this
   * attempt got its answer, throws `EffectKeyReusedError`, except that on a borrowed browser (ADR 0089)
   * a key declared again after its answer returns that answer, since the other crawler's retries reach
   * the Run as new calls. While the body runs in an attempt, staging an output with it (`enqueue`,
   * `addSeed`, `ctx.push`, a Key-value store write) throws `EffectStagingRefusedError`: stage it after the declaration,
   * from the output. A body that throws before the engine sent anything throws its own error unchanged; one that throws after a send throws
   * `EffectUncertainError`, chaining it, unless `repeatable`. An abort, or the end of the attempt
   * (`AttemptEndedError`), propagates unchanged, and the next declaration of the key acts on what was
   * sent. The program's own I/O inside the body is invisible to the journal. No `timeout` or
   * `signal`: the body's own calls carry theirs (ADR 0080).
   */
  effect<T extends EffectOutput>(key: string, body: (effect: Effect) => Promise<T>, options?: EffectOptions): Promise<EffectResult<T>>;
}

/**
 * Why an Effect is uncertain: its body threw after something was sent (`failed_after_send`), an
 * earlier attempt ended inside it after a send (`interrupted`), or the Run resumed from a Checkpoint
 * after which it may have gone on and the journal lacks the key (`journal_incomplete`, ADR 0002).
 */
export type UncertainWhy = "failed_after_send" | "interrupted" | "journal_incomplete" | (string & {});

/** An Effect the journal holds as uncertain, until a person reconciles it (ADR 0029). */
export interface UncertainEffect {
  readonly journalId: string;
  /** A declared Effect's key; `null` for an AI action. */
  readonly key: string | null;
  /** An AI action's instruction (ADR 0091); `null` for a declared Effect. */
  readonly instruction: string | null;
  /** The crawl request whose namespace holds it; `null` outside a handler. */
  readonly request: CrawlRequestRef | null;
  readonly why: UncertainWhy;
  /** The first command sent for it, in this language, such as `await page.getByRole('button', { name: 'Place order' }).click();`; `null` when the journal lost it. */
  readonly firstSent: string | null;
  readonly since: Date;
}

/** `effect.*` (ADR 0106): set on a connection or a Run. */
export interface EffectSettings {
  readonly output?: {
    /** `effect.output.max_size`, in bytes: the largest output the journal keeps. Built-in: 64 KiB. */
    readonly maxSize?: number;
  };
}

// The Error codes of declared Effects (ADR 0079's naming rule). `effect_uncertain` is ADR 0091's (ai.d.ts).

/** An empty key, or one longer than 255 characters. */
export declare class EffectKeyInvalidError extends ProgramError {
  readonly code: "effect_key_invalid";
  readonly details: { readonly key: string };
}

/** A key declared again while its body runs or after this attempt got its answer: a loop whose key lacks its varying part. */
export declare class EffectKeyReusedError extends ProgramError {
  readonly code: "effect_key_reused";
  /** `running`: its body is still running in this attempt. */
  readonly details: { readonly key: string; readonly answer: "performed" | "journal" | "reconciled" | "running" | (string & {}) };
}

/**
 * The body returned a value that is not JSON or is larger than `effect.output.max_size`: thrown as
 * such when nothing was sent, and as the cause of `EffectUncertainError` after a send.
 */
export declare class EffectOutputInvalidError extends ProgramError {
  readonly code: "effect_output_invalid";
  readonly details: { readonly key: string; readonly problem: "not_json" | "too_large" | (string & {}); readonly size: number | null };
}

/**
 * A call staged an output with an attempt while a declared Effect's body ran in it: a re-run that gets
 * `journal` does not run the body again, so the output would be lost when the attempt ends early.
 * `key` is the running body's.
 */
export declare class EffectStagingRefusedError extends ProgramError {
  readonly code: "effect_staging_refused";
  readonly details: { readonly key: string; readonly staged: "enqueue" | "seed" | "record" | "key_value" | (string & {}) };
}
