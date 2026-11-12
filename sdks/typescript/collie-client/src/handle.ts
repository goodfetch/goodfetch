// What a generated handle reaches its engine through (ADR 0099): the raw
// call, the ID its calls carry, and the host of each handle it opens or
// builds. Hand-written, since no catalog entry shapes it; the client
// package's runtime implements it.
import type { Caller, OperationTypes } from "./caller.ts";

/** What a handle of ID `Id` reaches its engine through. */
export interface HandleHost<
  Id,
  Types extends Record<keyof Types, OperationTypes>,
> extends Caller<Types> {
  /** The ID each of the handle's calls carries. */
  readonly id: Id;
  /** The host of a handle this one opens or builds, whose calls carry `id`. */
  child<ChildId>(id: ChildId): HandleHost<ChildId, Types>;
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
