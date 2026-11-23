// A program's own exception, as the errors that wrap it name it (ADRs
// 0079 and 0082): `handler_failed` and `schema_validator_failed` always
// carry the exception's type and message, whatever the program threw.

/** The type and message of what a program's own code threw. */
export interface ThrownException {
  /** An `Error`'s `name`; `null`, or the `typeof` of any other value. */
  readonly type: string;
  /** An `Error`'s `message`, or any other value as a string. */
  readonly message: string;
}

/** The type and message of `thrown`, which may be any value, not only an `Error`. */
export function thrownException(thrown: unknown): ThrownException {
  if (thrown instanceof Error) {
    return { type: thrown.name, message: thrown.message };
  }
  return { type: thrown === null ? "null" : typeof thrown, message: String(thrown) };
}
