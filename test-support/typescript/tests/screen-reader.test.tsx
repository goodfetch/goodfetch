// Seam B (spec #326): `virtualScreenReader(container)` starts
// `@guidepup/virtual-screen-reader` over a component until the test ends
// (ADR 0073). Its spoken log gives each element's role and accessible name,
// so a test catches an unnamed control, and it announces a live region's
// change.
import { expect, onTestFinished, test } from "vitest";
import { virtualScreenReader } from "@goodfetch/test-support/vitest-browser";

/** Renders `html` in a container of its own, removed when the test ends. */
function mount(html: string): HTMLElement {
  const container = document.createElement("div");
  container.innerHTML = html;
  document.body.append(container);
  onTestFinished(() => container.remove());
  return container;
}

test("reads a component's roles and names in order", async () => {
  const container = mount(
    `<nav aria-label="Runs"><h2>Runs</h2><button type="button">Start a run</button><button type="button" aria-label="Delete the run"><svg aria-hidden="true" width="8" height="8"></svg></button></nav>`,
  );
  const reader = await virtualScreenReader(container);

  for (let step = 0; step < 4; step += 1) {
    await reader.next();
  }

  expect(await reader.spokenPhraseLog()).toEqual([
    "navigation, Runs",
    "heading, Runs, level 2",
    "button, Start a run",
    "button, Delete the run",
    "end of navigation, Runs",
  ]);
});

test("reads an icon button without a name as a bare button", async () => {
  const container = mount(
    `<button type="button"><svg aria-hidden="true" width="8" height="8"></svg></button>`,
  );
  const reader = await virtualScreenReader(container);

  expect(await reader.spokenPhraseLog()).toEqual(["button"]);
});

test("announces a live region's change", async () => {
  const container = mount(`<button type="button">Start a run</button><p role="status"></p>`);
  const status = container.querySelector("p");
  container.querySelector("button")?.addEventListener("click", () => {
    if (status !== null) {
      status.textContent = "The run started";
    }
  });
  const reader = await virtualScreenReader(container);

  await reader.act();

  await expect.poll(() => reader.spokenPhraseLog()).toContain("polite: The run started");
});
