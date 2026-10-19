// The virtual screen reader of the `component` project (ADR 0073):
// `@guidepup/virtual-screen-reader` over the DOM, for tests that assert the
// roles and names a screen reader speaks, live region announcements among
// them. Real screen readers run as the `screen-reader` Live check.
import { Virtual } from "@guidepup/virtual-screen-reader";
import { onTestFinished } from "vitest";

/**
 * Starts a virtual screen reader of its own over `container` (the page's
 * body when left out), which stops when the test ends. Move it with
 * `next()` and `previous()`, act with `act()` or `press(key)`, and assert
 * `spokenPhraseLog()`: each entry is the role, the accessible name and its
 * states, and a live region's change is announced as `polite: …` or
 * `assertive: …`.
 */
export async function virtualScreenReader(container: Node = document.body): Promise<Virtual> {
  const reader = new Virtual();
  await reader.start({ container });
  onTestFinished(() => reader.stop());
  return reader;
}
