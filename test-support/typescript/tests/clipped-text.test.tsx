// Seam B (spec #326): `clippedText(locator)` fails with `text_clipped` when an
// `overflow` box cuts off text under the locator with no way to read it in
// full (ADR 0073), and passes text a reader can scroll to and visually hidden
// text. `textSpacing()` applies WCAG's text-spacing values and 200% text to
// the test's page until the test ends.
import { expect, onTestFinished, test } from "vitest";
import { page } from "vitest/browser";
import { clippedText, textSpacing } from "@goodfetch/test-support/vitest-browser";

/** Renders `html` in a container of its own, removed when the test ends. */
function mount(html: string): HTMLElement {
  const container = document.createElement("div");
  container.innerHTML = html;
  document.body.append(container);
  onTestFinished(() => container.remove());
  return container;
}

const FIXED_BOX = "width: 160px; height: 24px; overflow: hidden";

test("passes a label that wraps inside its box", async () => {
  const container = mount(`<div style="width: 80px">Start a run on every page of the site</div>`);

  await expect(clippedText(page.elementLocator(container))).resolves.toBeUndefined();
});

test("fails with text_clipped on a label its box cuts off", async () => {
  const container = mount(
    `<div style="width: 80px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis">Start a run on every page of the site</div>`,
  );

  await expect(clippedText(page.elementLocator(container))).rejects.toMatchObject({
    code: "text_clipped",
  });
});

test("passes a label in a fixed box at the default text spacing", async () => {
  const container = mount(`<div style="${FIXED_BOX}">Save</div>`);

  await expect(clippedText(page.elementLocator(container))).resolves.toBeUndefined();
});

test("fails with text_clipped on the same label under WCAG's text spacing and 200% text", async () => {
  const container = mount(`<div style="${FIXED_BOX}">Save</div>`);
  textSpacing();

  await expect(clippedText(page.elementLocator(container))).rejects.toMatchObject({
    code: "text_clipped",
  });
});

test("passes text a region lets the reader scroll to", async () => {
  const container = mount(
    `<div role="region" aria-label="Log" tabindex="0" style="width: 80px; height: 40px; overflow: auto; white-space: nowrap">Fetched every page of the site in one run</div>`,
  );

  await expect(clippedText(page.elementLocator(container))).resolves.toBeUndefined();
});

test("passes visually hidden text", async () => {
  const container = mount(
    `<button type="button">×<span style="position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); white-space: nowrap">Close the dialog</span></button>`,
  );

  await expect(clippedText(page.elementLocator(container))).resolves.toBeUndefined();
});
