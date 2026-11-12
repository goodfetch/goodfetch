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
  /** A shared fixture of `test-support/fixtures/` fails its committed schema. */
  | "fixture_invalid"
  /** A Secret canary search could not read one of its outputs (ADR 0067). */
  | "canary_output_unreadable"
  /** The description `goodfetch-test-env` printed fails its committed schema. */
  | "description_invalid"
  /** A test took `provingGround` without setting `provingGroundSeed`. */
  | "proving_ground_unseeded"
  /** A test took `daemon` without setting `withDaemon`. */
  | "daemon_not_requested"
  /** A Playwright test gave its context a `storageState`, which no test may set (ADR 0068). */
  | "storage_state_refused"
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
  | "page_errors"
  /** A known issue of a test tool was tolerated while another one was (ADR 0068). */
  | "known_issue_nested"
  /** `features/inventory.json` cannot be read, or fails its committed schema (ADR 0072). */
  | "inventory_invalid"
  /** A suite's case list cannot be read, is no TOML, or lacks its suite's name and version, a capability's name, or a case's ID or `covers` (ADRs 0033 and 0069). */
  | "case_list_invalid"
  /** A case log entry fails the bundle's `case_log_entry` schema, such as by setting an outcome (ADR 0069). */
  | "case_log_entry_invalid"
  /** A test carries a `client_case/` tag naming no case of the reporter's case list (ADR 0069). */
  | "client_case_unknown"
  /** A test took `clientCase` without carrying a `client_case/` tag (ADR 0069). */
  | "client_case_untagged"
  /** A Playwright test's `covers` annotation names a Feature the inventory does not list (ADR 0072). */
  | "covers_unknown"
  /** axe found a violation of WCAG 2.2 AA or a best practice on a served test's page (ADR 0073). */
  | "axe_violations"
  /** `browserStateReset` lifted a test's overrides and the page still read them past its deadline (ADR 0068). */
  | "browser_state_not_reset"
  /** A stop's area changes while it stays unfocused, so `focusChecks` cannot tell a focus ring from it (ADR 0073). */
  | "focus_not_visible_unstable"
  /** At 320 × 256 CSS pixels, the page or a region of it scrolls horizontally outside a data table or code block (ADR 0073). */
  | "horizontal_scroll"
  /** Text is clipped by its box with no way to read it in full, such as by scrolling (ADR 0073). */
  | "text_clipped"
  /** `noRunningAnimations` ran on a page whose `prefers-reduced-motion` is not `reduce` (ADR 0073). */
  | "reduced_motion_off"
  /** An animation or transition was still running under `prefers-reduced-motion: reduce` (ADR 0073). */
  | "animations_running"
  /** `GOODFETCH_VISUAL_IMAGE` names another image than ci/visual/image.toml pins (ADR 0073). */
  | "visual_image_not_pinned"
  /** `GOODFETCH_VISUAL_BROWSERS` names a browser the pinned image does not ship, or none (ADR 0073). */
  | "visual_browser_unknown"
  /** A repeatability measurement's Playwright run wrote no report it can read (ADR 0073). */
  | "repeatability_unmeasured"
  /** A screenshot's page lacks a fixed setting: instant, locale, time zone, viewport, scale or motion (ADR 0073). */
  | "screenshot_unsettled"
  /** The Client sent the Scripted engine a message no step of its script expects (ADR 0069). */
  | "unexpected_message"
  /** The Client closed its end, or the script ended, with a step of the Scripted engine's script unused (ADR 0069). */
  | "unused_step"
  /** A `reply` or `await_reply` step of a Scripted engine's script follows no request to answer (ADR 0069). */
  | "nothing_to_answer";

/** An error of test support, matched on its `code`. */
export class TestSupportError extends Error {
  readonly code: TestSupportErrorCode;

  constructor(code: TestSupportErrorCode, message: string, options?: ErrorOptions) {
    super(message, options);
    // The code in the name puts it on every report line that shows the
    // error, such as Playwright's, where only the message and stack travel.
    this.name = `TestSupportError(${code})`;
    this.code = code;
  }
}
