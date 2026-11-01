// The known issues of the test tools (ADR 0068: a tool failure is replaced
// or reported, never worked around in the product): page errors a tool's own
// injected code raises, which `failOnErrors` tolerates only around the one
// action that raises them (`PageErrorWatch.tolerating`), only on the browser
// they were seen on, and only while the Playwright version they were seen
// on runs. A Vitest test fails when the installed Playwright moves, so a bump
// checks each entry again: drop it once the fault is gone, or record the new
// version once it is seen there.
import { createRequire } from "node:module";
import type { Page } from "@playwright/test";
import { watchOf, type KnownIssue, type PageError } from "./page-errors.ts";

/** A known issue of Playwright on one browser. */
export interface ToolIssue extends KnownIssue {
  /** The browser it happens on, as `browserType().name()` names it. */
  readonly browser: "chromium" | "firefox" | "webkit";
  /** The Playwright version it was seen on; on any other it is not tolerated. */
  readonly playwright: string;
}

/** The text WebKit logs for an inline style its page's CSP refuses. */
const WEBKIT_REFUSED_STYLE =
  "Refused to apply a stylesheet because its hash, its nonce, or 'unsafe-inline' does not appear in the style-src directive of the Content Security Policy.";

/** A `style-src-elem` refusal of inline code from no source file: code no page script inserted. */
const UNSOURCED_INLINE_STYLE = /^style-src-elem refused inline at :\d+$/u;

/**
 * Before every screenshot on WebKit, Playwright inserts `<style>body {}</style>`
 * from its utility world to sync animations (`inPagePrepareForScreenshots`,
 * `shouldToggleStyleSheetToSyncAnimations`), and the page's CSP refuses it:
 * one `style-src-elem` violation with no source file, its console message
 * and its report. The pixels are unaffected (20 of 20 repeated, #389).
 */
export const WEBKIT_SCREENSHOT_INLINE_STYLE: ToolIssue = {
  id: "webkit-screenshot-inline-style",
  browser: "webkit",
  playwright: "1.64.1",
  upstream:
    "not reported upstream yet: a Playwright issue (https://github.com/microsoft/playwright/issues) asking that WebKit's screenshot sync its animations without inserting a style the page's CSP refuses",
  matches: (error: PageError) => {
    switch (error.kind) {
      case "console":
        return error.text === WEBKIT_REFUSED_STYLE;
      case "csp_violation":
        return UNSOURCED_INLINE_STYLE.test(error.text);
      case "csp_report":
        return isUnsourcedInlineStyleReport(error.text);
      default:
        return false;
    }
  },
};

/** The known issues of the shared screenshot helper. */
const SCREENSHOT_ISSUES: readonly ToolIssue[] = [WEBKIT_SCREENSHOT_INLINE_STYLE];

/** The Playwright version the tests run on. */
const PLAYWRIGHT_VERSION: unknown = createRequire(import.meta.url)(
  "@playwright/test/package.json",
).version;

/**
 * Runs `action`, a screenshot of `page`, tolerating on the errors watch of
 * its context the known issue of its browser and Playwright version, if any.
 */
export async function toleratingToolIssues<T>(page: Page, action: () => Promise<T>): Promise<T> {
  const watch = watchOf(page.context());
  const browser = page.context().browser()?.browserType().name();
  const issue = SCREENSHOT_ISSUES.find(
    (known) => known.browser === browser && known.playwright === PLAYWRIGHT_VERSION,
  );
  if (watch === undefined || issue === undefined) {
    return action();
  }
  return watch.tolerating(issue, action);
}

/** Whether a CSP report's body refuses inline code under `style-src-elem` with no source file. */
function isUnsourcedInlineStyleReport(body: string): boolean {
  let parsed: unknown;
  try {
    parsed = JSON.parse(body);
  } catch {
    return false;
  }
  const report: unknown =
    typeof parsed === "object" && parsed !== null ? Reflect.get(parsed, "csp-report") : undefined;
  if (typeof report !== "object" || report === null) {
    return false;
  }
  return (
    Reflect.get(report, "effective-directive") === "style-src-elem" &&
    Reflect.get(report, "blocked-uri") === "inline" &&
    !Reflect.has(report, "source-file")
  );
}
