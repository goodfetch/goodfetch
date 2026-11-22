// The result of AI work that can claim success, an Agent's task or a Plan
// replayed toward a goal (ADRs 0079, 0100 and 0117): how it ended, its
// Verdict and what its model claimed, read apart, with the answer inside how
// it ended. `requireVerified()` turns a result its checks did not prove into
// `not_verified`, only when the program asks.
import { runtimeMessage } from "./goodfetch-error.ts";
import { NotVerifiedError } from "./generated/errors.ts";
import type { AxisReason, Reported, StopReason, Verdict } from "./generated/types.ts";
import { collieWire } from "./generated/wire.ts";
import { toWire } from "./wire.ts";

/**
 * What a stopped task had gathered (ADR 0100): `Partial<T>` for an object
 * output, the elements gathered so far for a list.
 */
export type PartialOf<T> = T extends readonly (infer Item)[] ? Item[] : Partial<T>;

/**
 * How AI work ended (ADR 0117), narrowed on `kind`: `finished` holds a
 * complete output, `stopped` its reason and what it had gathered.
 */
export type Ended<T> =
  | { readonly kind: "finished"; readonly output: T }
  | { readonly kind: "stopped"; readonly reason: StopReason; readonly partial: PartialOf<T> };

/** The fields of an AI work result. */
export interface AIWorkFields<T> {
  /** How the work ended, its answer inside. */
  readonly ended: Ended<T>;
  /** Whether the program's checks prove the work, never set by the model's claim. */
  readonly verdict: Verdict;
  /** Why the Verdict is not `verified`, empty when it is. */
  readonly verdictReasons: readonly AxisReason[];
  /**
   * What the model claimed, absent when no model claimed anything: the work
   * stopped before a claim, or was replayed whole.
   */
  readonly reported?: Reported | undefined;
}

const NOT_PROVEN =
  "Read the result's `ended` and `verdict` to use an output its checks did not prove, or fix what the details' `reasons` name; the model's claim is never proof.";

/** AI work's result (ADR 0079). */
export class AIWorkResult<T> implements AIWorkFields<T> {
  readonly ended: Ended<T>;
  readonly verdict: Verdict;
  readonly verdictReasons: readonly AxisReason[];
  readonly reported: Reported | undefined;

  constructor(fields: AIWorkFields<T>) {
    this.ended = fields.ended;
    this.verdict = fields.verdict;
    this.verdictReasons = fields.verdictReasons;
    this.reported = fields.reported;
  }

  /**
   * The output, only when the work finished and its Verdict is `verified`
   * (ADR 0079), as httpx's `raise_for_status()` turns an outcome into an
   * error only when asked.
   *
   * @throws {NotVerifiedError} when the work stopped or its Verdict is
   * anything but `verified`, with the Verdict, its reasons and the model's
   * claim in its details.
   */
  requireVerified(): T {
    if (this.ended.kind === "finished" && this.verdict === "verified") {
      return this.ended.output;
    }
    const details: Record<string, unknown> = {
      verdict: this.verdict,
      reasons: toWire(this.verdictReasons, { items: { type: "AxisReason" } }, collieWire.types),
    };
    if (this.reported !== undefined) {
      details["reported"] = this.reported;
    }
    throw new NotVerifiedError(
      runtimeMessage(
        `the AI work's Verdict is \`${this.verdict}\`, so its output is not verified`,
        {
          fix: NOT_PROVEN,
        },
      ),
      { details, fix: NOT_PROVEN },
    );
  }
}
