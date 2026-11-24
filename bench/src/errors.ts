// Bench's typed errors: an attempt's status comes from its error's code, never
// from the message text, which is for people only.

/** Every code a bench error carries. */
export type BenchErrorCode =
  /** The adapter did not answer a call in the campaign's time. */
  | "adapter_timeout"
  /** The adapter's process exited with calls pending. */
  | "adapter_exit"
  /** A call was made after the adapter was closed. */
  | "adapter_closed"
  /** The adapter answered a call with an error of its own. */
  | "adapter_error"
  /** The adapter wrote a line that is not a protocol message. */
  | "adapter_protocol"
  /** The adapter wrote more than the output limit. */
  | "output_limit"
  /** A sibling call timed out, so the shared process was stopped under this one. */
  | "sibling_abort"
  /** The adapter's `hello` or `prepare` failed. */
  | "startup_failed"
  /** The adapter was restarted more times than the campaign allows. */
  | "restart_limit"
  /** The adapter's answer to `run` does not have the protocol's shape. */
  | "invalid_output"
  /** More calls are pending on one adapter than it accepts. */
  | "capacity"
  /** The scheduler started the attempt past the campaign's deadline. */
  | "deadline"
  /** The adapter does not claim the task's kind. */
  | "capability"
  /** The host was refused for the class the campaign asks for. */
  | "host_gate"
  /** The Proving ground refused the submission or could not be reached. */
  | "oracle_refused"
  /** The campaign names a task the Proving ground does not list. */
  | "task_unknown"
  /** A file or answer read from outside does not have its schema's shape. */
  | "invalid_document";

/** An error of the benchmark harness, with its code. */
export class BenchError extends Error {
  readonly code: BenchErrorCode;

  constructor(code: BenchErrorCode, message: string, options?: ErrorOptions) {
    super(message, options);
    this.code = code;
    this.name = "BenchError";
  }
}

/** The code of any error: a bench error's own, `adapter_error` for any other. */
export function errorCode(error: unknown): BenchErrorCode {
  return error instanceof BenchError ? error.code : "adapter_error";
}

/**
 * An attempt's status: the oracle's outcome when the adapter answered, or
 * what its error's code says when it did not.
 */
export type AttemptStatus =
  | "verified"
  | "incomplete"
  | "false_success"
  | "failed"
  | "timeout"
  | "crashed"
  | "aborted"
  | "unsupported"
  | "rejected"
  | "skipped_gate";

/** The status of an attempt that failed with `code`. */
export function statusFor(code: BenchErrorCode): AttemptStatus {
  switch (code) {
    case "adapter_timeout":
      return "timeout";
    case "sibling_abort":
      return "aborted";
    case "adapter_exit":
    case "adapter_closed":
    case "adapter_protocol":
    case "output_limit":
    case "startup_failed":
    case "restart_limit":
      return "crashed";
    case "capacity":
    case "deadline":
      return "rejected";
    case "capability":
      return "unsupported";
    case "adapter_error":
    case "invalid_output":
    case "host_gate":
    case "oracle_refused":
    case "task_unknown":
    case "invalid_document":
      return "failed";
    default: {
      const exhaustive: never = code;
      return exhaustive;
    }
  }
}
