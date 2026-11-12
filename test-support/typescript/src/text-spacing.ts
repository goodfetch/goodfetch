// WCAG's text-spacing values (1.4.12) and 200% text (1.4.4) as one style
// sheet, which a test of the `component` project applies to its page with
// `textSpacing()` (ADR 0073), with `clippedText` checking nothing is cut off.
// Served tests cannot take it: the production CSP refuses an injected style
// sheet, and Playwright has no option for either.
import { onTestFinished } from "vitest";

/**
 * Line height 1.5 times the font size, paragraph spacing 2 times, letter
 * spacing 0.12 times and word spacing 0.16 times (WCAG 1.4.12), and the
 * root's text at 200% (WCAG 1.4.4), which scales every size given in `rem`,
 * `em` or `%`. A size given in `px` does not scale.
 */
export const TEXT_SPACING_STYLE: string = `
:root {
  font-size: 200% !important;
}
* {
  line-height: 1.5 !important;
  letter-spacing: 0.12em !important;
  word-spacing: 0.16em !important;
}
p {
  margin-block-end: 2em !important;
}
`;

/** Applies `TEXT_SPACING_STYLE` to the page until the test ends. */
export function textSpacing(): void {
  const style = document.createElement("style");
  style.textContent = TEXT_SPACING_STYLE;
  document.head.append(style);
  onTestFinished(() => style.remove());
}
