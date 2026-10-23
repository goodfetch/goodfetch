// The translation-emulation script of ADR 0073: browser translators wrap
// the text nodes they translate in `<font>` elements, which crashes a page
// that keeps references to its text nodes and later removes one from the
// parent it put it in (react#11538). The Release suite's translation
// project adds the script to every served test's pages, which must then
// raise no uncaught error.
import type { BrowserContext } from "@playwright/test";

/**
 * Adds the translation-emulation script to every page `context` opens from
 * now on: from the document's start, each new text node that holds more
 * than white space is moved into a new `<font>` element in its place, as
 * page translators do. Text under `translate="no"`, and in `script`,
 * `style`, `textarea`, `title`, `noscript`, `option` and `template`, is left
 * alone, as translators leave it.
 */
export async function emulateTranslation(context: BrowserContext): Promise<void> {
  await context.addInitScript(wrapNewTextInFont);
}

/** Runs in the page from the document's start: the emulation itself. */
function wrapNewTextInFont(): void {
  const untranslated = new Set([
    "SCRIPT",
    "STYLE",
    "TEXTAREA",
    "TITLE",
    "NOSCRIPT",
    "OPTION",
    "TEMPLATE",
  ]);
  const wrappers = new WeakSet<Element>();
  const wrap = (text: Text): void => {
    const parent = text.parentElement;
    if (
      !(parent instanceof HTMLElement) ||
      wrappers.has(parent) ||
      untranslated.has(parent.tagName) ||
      !parent.translate ||
      text.data.trim() === ""
    ) {
      return;
    }
    // oxlint-disable-next-line typescript/no-deprecated -- page translators insert the obsolete `<font>`, which is what this script emulates.
    const font = document.createElement("font");
    wrappers.add(font);
    parent.insertBefore(font, text);
    font.append(text);
  };
  const visit = (node: Node): void => {
    if (node instanceof Text) {
      wrap(node);
      return;
    }
    const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT);
    const texts: Text[] = [];
    for (let next = walker.nextNode(); next !== null; next = walker.nextNode()) {
      if (next instanceof Text) {
        texts.push(next);
      }
    }
    texts.forEach(wrap);
  };
  new MutationObserver((records) => {
    for (const record of records) {
      record.addedNodes.forEach(visit);
    }
  }).observe(document, { childList: true, subtree: true });
}
