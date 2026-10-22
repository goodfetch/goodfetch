// Two tests of one file that share the Playwright provider's pointer: the
// first hovers a trigger and leaves the pointer on it; the second renders a
// trigger in the same place and expects it not to be hovered. They pass in
// either order only when the pointer is moved away before each test.
//
// The trigger stays clear of the two places the pointer rests without this
// file's hover: the viewport's top-left corner, where headless Chromium on
// Linux puts it before any input (and hovers what lies there), and the body's
// centre, where `hoverReset`'s unhover leaves it.
import { expect, test } from "vitest";
import { userEvent } from "vitest/browser";

function renderTrigger(): HTMLButtonElement {
  const trigger = document.createElement("button");
  trigger.textContent = "Trigger";
  trigger.style.cssText = "position: fixed; top: 100px; left: 100px; width: 200px; height: 100px;";
  document.body.replaceChildren(trigger);
  return trigger;
}

async function nextFrames(): Promise<void> {
  for (let frame = 0; frame < 2; frame += 1) {
    await new Promise((resolve) => requestAnimationFrame(resolve));
  }
}

test("hovering", async () => {
  const trigger = renderTrigger();

  await userEvent.hover(trigger);

  expect(trigger.matches(":hover")).toBe(true);
});

test("not hovered", async () => {
  const trigger = renderTrigger();

  await nextFrames();

  expect(trigger.matches(":hover")).toBe(false);
});
