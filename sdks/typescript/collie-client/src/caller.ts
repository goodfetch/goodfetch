// What sends a call and answers its result (ADR 0038): the interface every
// Product's generated operations take, which the client package's runtime
// implements. Hand-written, since no catalog entry shapes it.

/** The options of a call that can be cancelled (ADR 0038). */
export interface CallOptions {
  /** Cancels the call in flight when it aborts; a call the Client still queues is never sent. */
  readonly signal?: AbortSignal | undefined;
  /** The call's own timeout, which also bounds its wait in the Client's queue. */
  readonly deadline?: CallDeadline | undefined;
}

/**
 * A call's own timeout, which also bounds its wait in the Client's queue
 * under the engine's cap on calls in flight (ADR 0038).
 */
export interface CallDeadline {
  /**
   * What its `timeout` overrides: the operation's setting, such as
   * `timeout.navigation`, or `timeout`, its parameter, for a raw call,
   * whose setting the SDK does not know.
   */
  readonly key: string;
  /**
   * Its `timeout` in milliseconds, already checked by `timeoutOf`: `null`
   * for none, and left out for the setting's own, which the engine alone
   * knows, so a call queued without one waits for its place for as long as
   * it takes.
   */
  readonly value: number | null | undefined;
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
