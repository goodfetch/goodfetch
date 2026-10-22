// Screen-like tests that render a label and assert nothing about its text:
// the project's setup must check it.
import { onTestFinished, test } from "vitest";

/** Renders `html` in a container of its own, removed when the test ends. */
function mount(html: string): void {
  const container = document.createElement("div");
  container.innerHTML = html;
  document.body.append(container);
  onTestFinished(() => container.remove());
}

// oxlint-disable-next-line vitest/expect-expect -- the project's setup checks the page, which is what the fixture shows
test("a label its fixed box cuts off under 200% text", () => {
  mount(`<div style="width: 160px; height: 24px; overflow: hidden">Save</div>`);
});

// oxlint-disable-next-line vitest/expect-expect -- the project's setup checks the page, which is what the fixture shows
test("a label that wraps", () => {
  mount(`<div style="width: 80px">Start a run on every page of the site</div>`);
});
