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
  | "processes_alive"
  /** A keyboard walk's Tab came back to a stop before focus left the page (ADR 0073). */
  | "focus_trapped"
  /** Focus fell to the page's body during a keyboard walk (ADR 0073). */
  | "focus_lost"
  /** An element with an interactive role that a keyboard walk's Tab or Shift+Tab never reached (ADR 0073). */
  | "focus_unreachable"
  /** The keyboard did not open the overlay a stop says it opens, through `aria-haspopup` or `popovertarget` (ADR 0073). */
  | "overlay_not_opened"
  /** Escape in an overlay did not give focus back to the overlay's trigger (ADR 0073). */
  | "focus_not_returned"
  /** A keyboard walk's stop looks the same focused and unfocused (ADR 0073). */
  | "focus_not_visible"
  /** A keyboard walk's stop is entirely covered by other content (ADR 0073). */
  | "focus_obscured"
  /** A browser command was asked for state its browser provider cannot set (ADR 0068). */
  | "provider_unsupported"
  /** A Playwright test turned on `bypassCSP`, which no test may set (ADR 0068). */
  | "bypass_csp_refused"
  /** A served test's page logged an error, threw, broke its CSP or had a same-origin request fail (ADR 0068). */
  | "page_errors";

/** An error of test support, matched on its `code`. */
export class TestSupportError extends Error {
  readonly code: TestSupportErrorCode;

  constructor(code: TestSupportErrorCode, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "TestSupportError";
    this.code = code;
  }
}
