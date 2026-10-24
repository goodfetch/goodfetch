// The setup file of the pseudo-locale projects (ADR 0073; the Release
// suite's `component-stories-en-XA` and `component-stories-ar-XB`, whose
// browser context takes the pseudo-locale as its locale): once each test
// ends, no text on its page is clipped, under `en-XA` every visible text
// is the catalog's or inside `translate="no"`, and under `ar-XB` the
// document's root runs right to left.
import { afterEach } from "vitest";
import { page } from "vitest/browser";
import { clippedText } from "./clipped-text.ts";
import { TestSupportError } from "./errors.ts";
import { untranslatedText } from "./untranslated-text.ts";

afterEach(async () => {
  const body = page.elementLocator(document.body);
  await clippedText(body);
  switch (navigator.language) {
    case "en-XA":
      await untranslatedText(body);
      break;
    case "ar-XB":
      if (document.documentElement.dir !== "rtl") {
        throw new TestSupportError(
          "direction_not_rtl",
          `Under ar-XB the document's root runs ${document.documentElement.dir || "ltr"}, not rtl.`,
        );
      }
      break;
    default:
      break;
  }
});
