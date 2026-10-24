// Seam B (spec #326): under the `en-XA` pseudo-locale (ADR 0073),
// `untranslatedText(locator)` fails with `text_untranslated` when visible
// text under the locator is neither pseudo-localized nor inside
// `translate="no"`: a string the catalog does not hold, or data a screen
// forgot to mark.
import { expect, onTestFinished, test } from "vitest";
import { page } from "vitest/browser";
import { untranslatedText } from "@goodfetch/test-support/vitest-browser";

/** Renders `html` in a container of its own, removed when the test ends. */
function mount(html: string): HTMLElement {
  const container = document.createElement("div");
  container.innerHTML = html;
  document.body.append(container);
  onTestFinished(() => container.remove());
  return container;
}

test("passes pseudo-localized text and data inside translate=no", async () => {
  const container = mount(
    `<p>[ Ŕũń <code translate="no">nightly</code> ƒĩńĩśĥēď ]</p><p translate="no">https://shop.test/<b>cart</b></p>`,
  );

  await expect(untranslatedText(page.elementLocator(container))).resolves.toBeUndefined();
});

test("passes the letters the pseudo-locale keeps, digits and punctuation", async () => {
  const container = mount(`<p>[ ēxţŕà 12, 3.5% — vĩēŵ ]</p>`);

  await expect(untranslatedText(page.elementLocator(container))).resolves.toBeUndefined();
});

test("fails with text_untranslated on a string the catalog does not hold", async () => {
  const container = mount(`<p>[ Ŕũńś ]</p><button>Apply</button>`);

  await expect(untranslatedText(page.elementLocator(container))).rejects.toMatchObject({
    code: "text_untranslated",
    message: expect.stringContaining('"Apply"'),
  });
});

test("fails with text_untranslated on data a message holds outside translate=no", async () => {
  const container = mount(`<p>[ Ŕũń nightly ƒĩńĩśĥēď ]</p>`);

  await expect(untranslatedText(page.elementLocator(container))).rejects.toMatchObject({
    code: "text_untranslated",
  });
});

test("passes text that is not shown", async () => {
  const container = mount(
    `<p hidden>Hidden</p><p style="display: none">None</p><script>const shown = false;</script>`,
  );

  await expect(untranslatedText(page.elementLocator(container))).resolves.toBeUndefined();
});
