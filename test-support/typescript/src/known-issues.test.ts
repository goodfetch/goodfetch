import { createRequire } from "node:module";
import { describe, expect, test } from "vitest";
import { WEBKIT_SCREENSHOT_INLINE_STYLE } from "./known-issues.ts";

const installedPlaywright: unknown = createRequire(import.meta.url)(
  "@playwright/test/package.json",
).version;

describe("WEBKIT_SCREENSHOT_INLINE_STYLE", () => {
  test("was seen on the installed Playwright, so a bump checks it again", () => {
    expect(WEBKIT_SCREENSHOT_INLINE_STYLE.playwright).toBe(installedPlaywright);
  });

  // The errors WebKit 2370 recorded for one screenshot of a page under the
  // production CSP (Playwright 1.64.1, the pinned image).
  test.for([
    {
      kind: "console",
      text: "Refused to apply a stylesheet because its hash, its nonce, or 'unsafe-inline' does not appear in the style-src directive of the Content Security Policy.",
    },
    { kind: "csp_violation", text: "style-src-elem refused inline at :5" },
    {
      kind: "csp_report",
      text: '{"type":"csp-violation","csp-report":{"document-uri":"http://127.0.0.1:1/a.html","violated-directive":"style-src-elem","effective-directive":"style-src-elem","blocked-uri":"inline","status-code":200}}',
    },
  ] as const)("matches the $kind WebKit records for Playwright's style", (error) => {
    const matched = WEBKIT_SCREENSHOT_INLINE_STYLE.matches({
      ...error,
      url: "http://127.0.0.1:1/",
    });

    expect(matched).toBe(true);
  });

  test.for([
    { kind: "csp_violation", text: "style-src-elem refused inline at http://127.0.0.1:1/a.js:6" },
    {
      kind: "csp_report",
      text: '{"csp-report":{"effective-directive":"style-src-elem","blocked-uri":"inline","source-file":"http://127.0.0.1:1/a.js","line-number":6}}',
    },
    { kind: "csp_violation", text: "script-src-elem refused inline at :5" },
    { kind: "page", text: "Error: boom" },
  ] as const)("does not match a $kind of another cause: $text", (error) => {
    const matched = WEBKIT_SCREENSHOT_INLINE_STYLE.matches({
      ...error,
      url: "http://127.0.0.1:1/",
    });

    expect(matched).toBe(false);
  });
});
