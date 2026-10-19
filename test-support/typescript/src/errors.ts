// The one error test support throws, told apart by its code (ADR 0038's
// rule for Error codes: a test asserts the code, never the message).

/** Why test support could not give a test its setup, or failed it at teardown. */
export type TestSupportErrorCode =
  /** `GOODFETCH_TEST_TOOLS` is not set: the tests run outside `just test`. */
  | "tools_unset"
  /** The test tool manifest cannot be read, or fails its committed schema. */
  | "tools_invalid"
  /** The test tool manifest lists no such executable. */
  | "tool_missing"
  /** `goodfetch-test-env` exited before it printed a description. */
  | "start_failed"
  /** The description `goodfetch-test-env` printed fails its committed schema. */
  | "description_invalid"
  /** `goodfetch-test-env` exited with a failure when its stdin closed. */
  | "teardown_failed"
  /** A process the test started through test support was alive after teardown (ADR 0069). */
  | "processes_alive";

/** An error of test support, matched on its `code`. */
export class TestSupportError extends Error {
  readonly code: TestSupportErrorCode;

  constructor(code: TestSupportErrorCode, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "TestSupportError";
    this.code = code;
  }
}
