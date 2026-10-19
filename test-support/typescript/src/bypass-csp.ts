// The production CSP is never relaxed in a test (ADR 0068): the browser keeps
// it too, so a test that turns on Playwright's `bypassCSP` fails.
import { TestSupportError } from "./errors.ts";

/** The options of a Playwright test that could turn on `bypassCSP`. */
export interface CspOptions {
  readonly bypassCSP?: boolean | undefined;
  readonly contextOptions?: { readonly bypassCSP?: boolean | undefined } | undefined;
}

/** Fails with `bypass_csp_refused` when `options` turn on `bypassCSP`, directly or through `contextOptions`. */
export function refuseBypassCSP(options: CspOptions): void {
  if (options.bypassCSP === true || options.contextOptions?.bypassCSP === true) {
    throw new TestSupportError(
      "bypass_csp_refused",
      "bypassCSP is on: every test runs under the production CSP (ADR 0068)",
    );
  }
}
