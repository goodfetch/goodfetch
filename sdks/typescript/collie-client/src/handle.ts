// What a generated handle reaches its engine through (ADR 0099): the raw
// call, the ID its calls carry, and the host of each handle it opens or
// builds. Hand-written, since no catalog entry shapes it; the client
// package's runtime implements it.
import type { Caller, OperationTypes } from "./caller.ts";
// The errors module itself, not the package index, which re-exports the
// handles that import this module.
import { SettingInvalidError } from "./generated/errors.ts";

/** What a handle of ID `Id` reaches its engine through. */
export interface HandleHost<
  Id,
  Types extends Record<keyof Types, OperationTypes>,
> extends Caller<Types> {
  /** The ID each of the handle's calls carries. */
  readonly id: Id;
  /** The host of a handle this one opens or builds, whose calls carry `id`. */
  child<ChildId>(id: ChildId): HandleHost<ChildId, Types>;
  /**
   * Closes the handle through `operation`, its own `close` (ADR 0078);
   * idempotent. It is sent once: a handle already closed, by its program or
   * with the value that opened it, sends nothing. Every other call on the
   * handle, and on a handle it opened or built, then fails with `closed`.
   */
  close<Name extends keyof Types & string>(
    operation: Name,
    params: Types[Name]["params"],
  ): Promise<void>;
}

/** `Params` without the entries a caller left out. */
export type Defined<Params> = {
  [Key in keyof Params]: Exclude<Params[Key], undefined>;
};

/**
 * `params` without its `undefined` entries: an optional parameter left out
 * is left out of the call, never sent as `undefined`.
 */
export function defined<Params extends object>(params: Params): Defined<Params> {
  const kept = Object.fromEntries(
    Object.entries(params).filter(([, value]) => value !== undefined),
  );
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the entries are `params`'s own, less those whose value is `undefined`, which is what `Defined` says.
  return kept as Defined<Params>;
}

/** What a refused call's `timeout` tells its program to give instead (ADR 0080). */
const TIMEOUT_FIX = "give at least 1 ms, or `timeout: null` for no timeout";

/**
 * A call's own `timeout`, sent as given once it can be a deadline (ADR
 * 0080): left out for `setting`, the Duration setting it sets for the call
 * alone, `null` for none, or a whole number of milliseconds of 1 or more.
 *
 * @throws {SettingInvalidError} before anything is sent, when `timeout` is
 * 0, negative, fractional or not finite: its `limit` names `setting` at the
 * run layer, given as an argument, and its `fix` the null form, as the
 * engine's refusal of `0` does.
 */
export function timeoutOf<Timeout extends number | null | undefined>(
  timeout: Timeout,
  setting: string,
): Timeout {
  if (
    timeout === undefined ||
    timeout === null ||
    (Number.isSafeInteger(timeout) && timeout >= 1)
  ) {
    return timeout;
  }
  const reason =
    timeout === 0
      ? "a timeout of 0 ms would end the call at once"
      : `a timeout of ${String(timeout)} ms is no whole number of milliseconds of 1 or more`;
  throw new SettingInvalidError(
    `The call's \`timeout\`, its \`${setting}\`: ${reason}; ${TIMEOUT_FIX}.`,
    {
      fix: TIMEOUT_FIX,
      details: {
        key: setting,
        layer: "run",
        reason,
        limit: { key: setting, value: timeout, layer: "run", origin: "argument" },
      },
    },
  );
}
