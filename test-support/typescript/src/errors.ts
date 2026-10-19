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
  /** A browser command was asked for state its browser provider cannot set (ADR 0068). */
  | "provider_unsupported"
  /** A Playwright test turned on `bypassCSP`, which no test may set (ADR 0068). */
  | "bypass_csp_refused"
  /** A served test's page logged an error, threw, broke its CSP or had a same-origin request fail (ADR 0068). */
  | "page_errors"
  /** axe found a violation of WCAG 2.2 AA or a best practice on a served test's page (ADR 0073). */
  | "axe_violations"
  /** An entry of the axe exception file names a selector that matches nothing on a page where its rule ran (ADR 0073). */
  | "axe_exception_unmatched"
  /** The axe exception file cannot be read, or an entry lacks a field or has one it does not define (ADR 0073). */
  | "axe_exceptions_invalid"
  /** At 320 × 256 CSS pixels, the page or a region of it scrolls horizontally outside a data table or code block (ADR 0073). */
  | "horizontal_scroll"
  /** Text is clipped by its box with no way to read it in full, such as by scrolling (ADR 0073). */
  | "text_clipped"
  /** `noRunningAnimations` ran on a page whose `prefers-reduced-motion` is not `reduce` (ADR 0073). */
  | "reduced_motion_off"
  /** An animation or transition was still running under `prefers-reduced-motion: reduce` (ADR 0073). */
  | "animations_running";

/** An error of test support, matched on its `code`. */
export class TestSupportError extends Error {
  readonly code: TestSupportErrorCode;

  constructor(code: TestSupportErrorCode, message: string, options?: ErrorOptions) {
    super(message, options);
    this.name = "TestSupportError";
    this.code = code;
  }
}
