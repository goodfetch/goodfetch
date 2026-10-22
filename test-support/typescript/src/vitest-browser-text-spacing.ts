// The setup file of the `component` project's text-spacing run (ADR 0073;
// the Release suite's `component-text-spacing` project): every test of the
// Console's screens runs with WCAG's text-spacing values and 200% text, and
// once it ends `clippedText` checks that nothing on its page is cut off
// under them, so a screen test fails on clipped text without calling it.
import { afterEach, beforeEach } from "vitest";
import { page } from "vitest/browser";
import { clippedText } from "./clipped-text.ts";
import { textSpacing } from "./text-spacing.ts";

beforeEach(textSpacing);
afterEach(() => clippedText(page.elementLocator(document.body)));
