// The setup file of the `component` project's text-spacing run (ADR 0073;
// the Release suite's `component-text-spacing` project): every test of the
// Console's screens runs with WCAG's text-spacing values and 200% text, and
// `clippedText` checks that nothing is cut off under them.
import { beforeEach } from "vitest";
import { textSpacing } from "./text-spacing.ts";

beforeEach(textSpacing);
