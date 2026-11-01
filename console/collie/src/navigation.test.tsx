// Moving between the Console's screens without loading the page again
// (ADR 0031: a page with no connection to the Daemon keeps reading what
// the offline cache keeps): a click on a link to the Console's own origin
// changes the address and tells the page, as going back does; a link to
// another origin, opened in another tab, or clicked with a modifier is the
// browser's, as before.
import { afterEach, expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { followLinks } from "./navigation.ts";

const start = window.location.href;
let stop: (() => void) | undefined;

afterEach(() => {
  stop?.();
  document.body.replaceChildren();
  window.history.replaceState(null, "", start);
});

/** Adds a link to `href` named `name`, with `target` when given. */
function link(name: string, href: string, target?: string): void {
  const anchor = document.createElement("a");
  anchor.href = href;
  anchor.textContent = name;
  if (target !== undefined) {
    anchor.target = target;
  }
  document.body.append(anchor);
}

test("a link to the Console's own origin changes the address without loading the page, and so does going back", async () => {
  const seen: string[] = [];
  stop = followLinks(window, () => seen.push(window.location.pathname));
  link("Runs", "/runs?text=shop");
  const before = window.location.pathname;

  await userEvent.click(page.getByRole("link", { name: "Runs" }));
  window.history.back();
  await expect.poll(() => seen).toEqual(["/runs", before]);

  expect(window.location.pathname).toBe(before);
});

test("a link to another origin, to another tab or clicked with a modifier is left to the browser", async () => {
  const seen: string[] = [];
  stop = followLinks(window, () => seen.push(window.location.pathname));
  const ignored: boolean[] = [];
  const recordDefault = (event: MouseEvent): void => {
    ignored.push(!event.defaultPrevented);
    event.preventDefault();
  };
  window.addEventListener("click", recordDefault);
  link("Elsewhere", "https://example.test/runs");
  link("New tab", "/runs", "_blank");
  link("Modified", "/runs");

  await userEvent.click(page.getByRole("link", { name: "Elsewhere" }));
  await userEvent.click(page.getByRole("link", { name: "New tab" }));
  await userEvent.click(page.getByRole("link", { name: "Modified" }), { modifiers: ["Shift"] });
  window.removeEventListener("click", recordDefault);

  expect(ignored).toEqual([true, true, true]);
  expect(seen).toEqual([]);
});
