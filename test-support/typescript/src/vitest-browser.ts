// What a Vitest Browser Mode test takes from test support (ADR 0068): the
// pointer reset the shared setup (`@goodfetch/test-support/vitest-browser/setup`)
// runs before each test, and the browser state a test sets, which lasts until
// the test ends. The browser commands they call are registered by the
// project's configuration from `@goodfetch/test-support/vitest-browser/commands`.
import { onTestFinished } from "vitest";
import { commands, userEvent } from "vitest/browser";
import type { BrowserState } from "./browser-state.ts";

export type { BrowserState } from "./browser-state.ts";
export { clippedText } from "./clipped-text.ts";
export { virtualScreenReader } from "./screen-reader.ts";
export { TEXT_SPACING_STYLE, textSpacing } from "./text-spacing.ts";
export { untranslatedText } from "./untranslated-text.ts";

declare module "vitest/browser" {
  interface BrowserCommands {
    browserState: (state: BrowserState) => Promise<void>;
    browserStateReset: () => Promise<void>;
  }
}

/** Moves the pointer to the page's body, off anything a test before hovered. */
export async function hoverReset(): Promise<void> {
  await userEvent.unhover(document.body);
}

/**
 * Sets the given browser state on the Playwright provider's page and context,
 * until the test ends, when `browserStateReset` gives the page back the
 * project's state. A locale, time zone or online status is refused outside
 * Chromium, where Playwright fixes them when the context is created: a
 * project sets them in its provider's context options instead.
 */
export async function browserState(state: BrowserState): Promise<void> {
  onTestFinished(browserStateReset);
  await commands.browserState(state);
}

/** Gives the page back the project's browser state, undoing `browserState`. */
export async function browserStateReset(): Promise<void> {
  await commands.browserStateReset();
}
