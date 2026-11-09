// What sends a call and answers its result (ADR 0038): the interface every
// Product's generated operations take, which the client package's runtime
// implements. Hand-written, since no catalog entry shapes it.

/** The options of a call that can be cancelled (ADR 0038). */
export interface CallOptions {
  /** Cancels the call in flight when it aborts. */
  readonly signal?: AbortSignal | undefined;
}

/** The params and result of one operation. */
export interface OperationTypes {
  readonly params: unknown;
  readonly result: unknown;
}

/**
 * What sends a call and answers its result: the client package's runtime.
 * Results are not validated beyond the protocol version check (STYLE.md).
 */
export interface Caller<Types extends Record<keyof Types, OperationTypes>> {
  call<Name extends keyof Types & string>(
    operation: Name,
    params: Types[Name]["params"],
    options: CallOptions,
  ): Promise<Types[Name]["result"]>;
}
