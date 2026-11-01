// The untranslated-text check of ADR 0073's `en-XA` pseudo-locale: under
// it, every message of the catalog is pseudo-localized (its Latin letters
// accented, bracketed and longer), so visible text that still holds a plain
// Latin letter outside `translate="no"` is a string the catalog does not
// hold, or data a screen forgot to mark. It runs in a Vitest Browser Mode
// test, usually through the pseudo-locale projects' setup file.
import type { Locator } from "vitest/browser";
import { TestSupportError } from "./errors.ts";

/**
 * The Latin letters a pseudo-localized message no longer holds: Lingui's
 * pseudo-localization (the `pseudolocale` package) accents every one but
 * `v`, `x`, `V` and `X`, which it keeps as they are.
 */
const PLAIN_LETTER = /[a-uwyzA-UWYZ]/u;

/** The elements whose text is never shown, as browsers and translators treat them. */
const UNSHOWN = new Set(["SCRIPT", "STYLE", "TEMPLATE", "NOSCRIPT", "TITLE"]);

/**
 * Fails with `text_untranslated` when visible text under the element
 * `locator` finds holds a plain Latin letter outside `translate="no"`,
 * under a catalog pseudo-localized as `en-XA`: such text did not come from
 * the catalog, or is data rendered without `translate="no"`. Text that is
 * not rendered or visible passes.
 */
export async function untranslatedText(locator: Locator): Promise<void> {
  const root = locator.element();
  const untranslated: string[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    const parent = node.parentElement;
    if (
      node instanceof Text &&
      parent instanceof HTMLElement &&
      PLAIN_LETTER.test(node.data) &&
      parent.translate &&
      !UNSHOWN.has(parent.tagName) &&
      parent.checkVisibility({ visibilityProperty: true })
    ) {
      untranslated.push(`"${node.data.trim().slice(0, 60)}", in <${parent.localName}>`);
    }
  }
  if (untranslated.length > 0) {
    throw new TestSupportError(
      "text_untranslated",
      `Visible text is neither from the catalog nor inside translate="no":\n${untranslated.map((line) => `- ${line}`).join("\n")}`,
    );
  }
}
