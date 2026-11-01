// The keyboard helpers of served tests (ADR 0073; TESTING.md, Accessibility):
// what axe cannot see about keyboard use, tested the same way on every
// screen.
//
// - `keyboardWalk(page)` tabs forward through the page, presses Escape in
//   each overlay a stop opens and checks focus comes back to the stop, tabs
//   back, and gives the stops' roles and names for a snapshot.
// - `focusChecks(page)` tabs forward and, at each stop, checks the stop is
//   not entirely covered and looks different focused and unfocused.
//
// A walk presses Tab, or Shift+Tab backwards, until focus leaves the page,
// which the page sees as `document.hasFocus()` turning false. Focus on the
// body while the page still has it means focus fell there. WebKit's Tab
// skips links and buttons, as macOS does by default, so on WebKit a walk
// presses Alt+Tab, which reaches them.
import { toleratingToolIssues } from "./known-issues.ts";
import { expect, type ElementHandle, type Page } from "@playwright/test";
import { TestSupportError, type TestSupportErrorCode } from "./errors.ts";

/** A keyboard walk's stop: the focused element's role and accessible name, both empty when the element is hidden from the accessibility tree. */
export interface FocusStop {
  readonly role: string;
  readonly name: string;
}

/**
 * Walks the page with the keyboard and fails when a rule of ADR 0073 breaks:
 * Tab comes back to a stop before focus leaves the page (`focus_trapped`),
 * focus falls to the body (`focus_lost`), Tab or Shift+Tab never reaches a
 * visible, enabled element with an interactive role (`focus_unreachable`), a
 * stop with `aria-haspopup` or `popovertarget` does not open its overlay on
 * Enter, or on ArrowDown for a combobox (`overlay_not_opened`), or Escape
 * there does not give focus back to the stop (`focus_not_returned`).
 *
 * The walk starts at the page's top, wherever focus was. It gives each
 * stop's role and accessible name in Tab order, for a snapshot. A member of
 * a composite widget (an option, a menu item, a tab, a tree item or a radio
 * button) is reached when its widget holds a stop, since arrow keys move
 * within it. Elements inside shadow roots and frames are not checked for
 * reach.
 */
export async function keyboardWalk(page: Page): Promise<readonly FocusStop[]> {
  // Tab out of the page from wherever focus is, so the next walk starts at its top.
  await walk(page, "forward");
  const stops: FocusStop[] = [];
  const forward = await walk(page, "forward", async (stop) => {
    stops.push(await focusedStop(page));
    await escapeOverlayOf(page, stop);
  });
  await expectEveryInteractiveElementReached(page, forward, "forward");
  const backward = await walk(page, "backward");
  await expectEveryInteractiveElementReached(page, backward, "backward");
  return stops;
}

/**
 * Walks the page forward and fails at the first stop that is entirely
 * covered by other content (`focus_obscured`: no point of a 5 × 5 grid over
 * it shows the stop on top through `elementsFromPoint`), whose screenshot,
 * with a margin of 8 CSS pixels for its focus ring, changes while it stays
 * unfocused (`focus_not_visible_unstable`: something there, such as an
 * animation that never ends, would pass for a focus ring), or is the same
 * focused and unfocused (`focus_not_visible`). A walk's own failures fail it
 * too. On WebKit, the style Playwright adds before each screenshot, which the
 * production CSP refuses, is tolerated as a known issue (`known-issues.ts`).
 */
export async function focusChecks(page: Page): Promise<void> {
  // Tab out of the page from wherever focus is, so the next walk starts at its top.
  await walk(page, "forward");
  await walk(page, "forward", async (stop) => {
    if (await stop.evaluate(isEntirelyCovered)) {
      throw new TestSupportError("focus_obscured", `${await describe(stop)} is entirely covered`);
    }
    await expectFocusVisible(page, stop);
  });
}

/** Fails with `focus_not_visible` when the area of the stop and its focus ring looks the same focused and unfocused. */
async function expectFocusVisible(page: Page, stop: ElementHandle<Element>): Promise<void> {
  const clip = await stop.evaluate(focusArea, FOCUS_RING_MARGIN);
  if (clip.width === 0 || clip.height === 0) {
    throw new TestSupportError("focus_not_visible", `${await describe(stop)} has no visible area`);
  }
  // Playwright's own `animations: "disabled"` and `caret: "hide"` add a style
  // element too; the page's animations are finished and the stop's caret
  // hidden through the CSSOM instead, which the CSP allows.
  const screenshot = { clip, animations: "allow", caret: "initial", scale: "css" } as const;
  const caret = await stop.evaluate(hideCaret);
  const shot = async (): Promise<Buffer> => {
    await page.evaluate(finishAnimations);
    return toleratingToolIssues(page, () => page.screenshot(screenshot));
  };
  let focused: Buffer;
  let unfocused: Buffer;
  let unfocusedAgain: Buffer;
  try {
    focused = await shot();
    await stop.evaluate(blur);
    unfocused = await shot();
    unfocusedAgain = await shot();
    await stop.evaluate(focus);
  } finally {
    await stop.evaluate(showCaret, caret);
  }
  if (!unfocused.equals(unfocusedAgain)) {
    throw new TestSupportError(
      "focus_not_visible_unstable",
      `the area of ${await describe(stop)} changes while it stays unfocused, so a change on focus would prove no focus ring; stop what moves there`,
    );
  }
  if (focused.equals(unfocused)) {
    throw new TestSupportError(
      "focus_not_visible",
      `${await describe(stop)} looks the same focused and unfocused`,
    );
  }
}

/** How far outside its box a stop's focus ring may be drawn, in CSS pixels. */
const FOCUS_RING_MARGIN = 8;

type Direction = "forward" | "backward";

/** What a walk does at a stop while it is focused, before the walk moves on. */
type AtStop = (stop: ElementHandle<Element>) => Promise<void>;

/** Presses Tab in `direction` until focus leaves the page, and gives the stops in order. */
async function walk(
  page: Page,
  direction: Direction,
  atStop?: AtStop,
): Promise<ElementHandle<Element>[]> {
  const key = tabKey(page, direction);
  const limit = await page.evaluate(() => 2 * document.querySelectorAll("*").length + 10);
  const stops: ElementHandle<Element>[] = [];
  for (let press = 0; press < limit; press += 1) {
    await page.keyboard.press(key);
    const stop = (await page.evaluateHandle(focusedElement)).asElement();
    if (stop === null) {
      if (await page.evaluate(() => document.hasFocus())) {
        const previous = stops.at(-1);
        const after = previous === undefined ? "the walk's start" : await describe(previous);
        throw new TestSupportError("focus_lost", `focus fell to the body after ${after} (${key})`);
      }
      return stops;
    }
    if (await stop.evaluate((element, seen) => seen.includes(element), stops)) {
      throw new TestSupportError(
        "focus_trapped",
        `${key} came back to ${await describe(stop)} before focus left the page`,
      );
    }
    stops.push(stop);
    await atStop?.(stop);
  }
  throw new TestSupportError(
    "focus_trapped",
    `focus did not leave the page after ${limit} presses of ${key}`,
  );
}

function tabKey(page: Page, direction: Direction): string {
  const tab = browserName(page) === "webkit" ? "Alt+Tab" : "Tab";
  return direction === "forward" ? tab : `Shift+${tab}`;
}

/** `chromium`, `firefox` or `webkit`. */
function browserName(page: Page): string | undefined {
  return page.context().browser()?.browserType().name();
}

/** The focused element through open shadow roots, or `null` when focus is on the body or nowhere. Runs in the page. */
function focusedElement(): Element | null {
  let focused = document.activeElement;
  while (focused?.shadowRoot?.activeElement) {
    focused = focused.shadowRoot.activeElement;
  }
  return focused === null || focused === document.body || focused === document.documentElement
    ? null
    : focused;
}

/** The focused element's role and name, from the first line of its ARIA snapshot (`- button "Save"`). */
async function focusedStop(page: Page): Promise<FocusStop> {
  const snapshot = await page.locator("*:focus").last().ariaSnapshot();
  const match = /^- ([\w-]+)(?: "((?:[^"\\]|\\.)*)")?/.exec(snapshot);
  const role = match?.[1];
  if (role === undefined) {
    return { role: "", name: "" };
  }
  const name: unknown = JSON.parse(`"${match?.[2] ?? ""}"`);
  return { role, name: typeof name === "string" ? name : "" };
}

/** Opens the overlay a stop says it opens, presses Escape and checks focus is back on the stop. */
async function escapeOverlayOf(page: Page, trigger: ElementHandle<Element>): Promise<void> {
  const opener = await trigger.evaluate(overlayOpener);
  if (opener === null) {
    return;
  }
  await page.keyboard.press(opener);
  await eventually(
    () => trigger.evaluate(isOverlayOpen),
    "overlay_not_opened",
    async () => `${opener} on ${await describe(trigger)} opened no overlay`,
  );
  await page.keyboard.press("Escape");
  await eventually(
    () => trigger.evaluate(isFocused),
    "focus_not_returned",
    async () => `Escape did not give focus back to ${await describe(trigger)}`,
  );
}

/** Waits, as `expect.poll` does, until `check` holds, and fails with `code` when it never does. */
async function eventually(
  check: () => Promise<boolean>,
  code: TestSupportErrorCode,
  message: () => Promise<string>,
): Promise<void> {
  try {
    await expect.poll(check).toBe(true);
  } catch (error) {
    throw new TestSupportError(code, await message(), { cause: error });
  }
}

/** The key that opens the overlay an element says it opens, or `null` when it opens none. Runs in the page. */
function overlayOpener(element: Element): "Enter" | "ArrowDown" | null {
  const popup = element.getAttribute("aria-haspopup");
  if ((popup === null || popup === "false") && !element.hasAttribute("popovertarget")) {
    return null;
  }
  return element.getAttribute("role") === "combobox" ? "ArrowDown" : "Enter";
}

/** Whether the overlay a trigger opens is open: focus left the trigger, it is expanded, or its popover shows. Runs in the page. */
function isOverlayOpen(trigger: Element): boolean {
  let focused = document.activeElement;
  while (focused?.shadowRoot?.activeElement) {
    focused = focused.shadowRoot.activeElement;
  }
  const popover = trigger.getAttribute("popovertarget");
  return (
    focused !== trigger ||
    trigger.getAttribute("aria-expanded") === "true" ||
    (popover !== null && document.getElementById(popover)?.matches(":popover-open") === true)
  );
}

/** Whether the element has focus, through open shadow roots. Runs in the page. */
function isFocused(element: Element): boolean {
  let focused = document.activeElement;
  while (focused?.shadowRoot?.activeElement) {
    focused = focused.shadowRoot.activeElement;
  }
  return focused === element;
}

async function expectEveryInteractiveElementReached(
  page: Page,
  stops: readonly ElementHandle<Element>[],
  direction: Direction,
): Promise<void> {
  const unreached = await page.evaluate(unreachedElements, stops);
  if (unreached.length > 0) {
    throw new TestSupportError(
      "focus_unreachable",
      `${tabKey(page, direction)} never reached ${unreached.join(", ")}`,
    );
  }
}

/** Each visible, enabled element with an interactive role that no stop reaches, described. Runs in the page. */
/* oxlint-disable unicorn/consistent-function-scoping -- Playwright sends only this function's source to the page, so its helpers must be inside it. */
function unreachedElements(stops: readonly Element[]): string[] {
  const interactive = new Set([
    "button",
    "checkbox",
    "combobox",
    "input",
    "link",
    "listbox",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "radio",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "textbox",
    "treeitem",
  ]);
  const composite = [
    "listbox",
    "menu",
    "menubar",
    "tablist",
    "tree",
    "treegrid",
    "grid",
    "radiogroup",
  ].map((role) => `[role="${role}"]`);
  const widgets = [...composite, "select", "datalist"].join(", ");
  const roleOf = (element: Element): string | null => {
    const explicit = element.getAttribute("role")?.trim().split(/\s+/)[0];
    if (explicit !== undefined && explicit !== "") {
      return explicit;
    }
    switch (element.localName) {
      case "a":
      case "area":
        return element.hasAttribute("href") ? "link" : null;
      case "button":
      case "summary":
        return "button";
      case "select":
        return "combobox";
      case "textarea":
        return "textbox";
      case "option":
        return "option";
      case "input": {
        const type = element.getAttribute("type")?.toLowerCase();
        return type === "hidden" ? null : type === "radio" ? "radio" : "input";
      }
      default:
        return element instanceof HTMLElement &&
          element.isContentEditable &&
          element.parentElement?.isContentEditable !== true
          ? "textbox"
          : null;
    }
  };
  const isLeftOut = (element: Element): boolean =>
    !element.checkVisibility({ visibilityProperty: true }) ||
    element.closest('[inert], [aria-hidden="true"]') !== null ||
    element.matches(":disabled") ||
    element.getAttribute("aria-disabled") === "true";
  const holdsStop = (container: Element): boolean => stops.some((stop) => container.contains(stop));
  const isReached = (element: Element): boolean => {
    if (stops.includes(element)) {
      return true;
    }
    const widget = element.parentElement?.closest(widgets);
    if (widget !== null && widget !== undefined) {
      return holdsStop(widget);
    }
    if (element instanceof HTMLInputElement && element.type === "radio" && element.name !== "") {
      const scope = element.form ?? document;
      const group = scope.querySelectorAll(
        `input[type="radio"][name="${CSS.escape(element.name)}"]`,
      );
      return [...group].some((radio) => stops.includes(radio));
    }
    return false;
  };
  return [...document.querySelectorAll("*")]
    .filter((element) => {
      const role = roleOf(element);
      return role !== null && interactive.has(role) && !isLeftOut(element) && !isReached(element);
    })
    .map((element) => {
      const text = (element.getAttribute("aria-label") ?? element.textContent ?? "")
        .trim()
        .replaceAll(/\s+/g, " ");
      return `${element.localName} "${text.slice(0, 60)}"`;
    });
}

/* oxlint-enable unicorn/consistent-function-scoping */

/** An element in a failure message: its tag and its label or text. */
async function describe(stop: ElementHandle<Element>): Promise<string> {
  return stop.evaluate((element) => {
    const text = (element.getAttribute("aria-label") ?? element.textContent ?? "")
      .trim()
      .replaceAll(/\s+/g, " ");
    return `${element.localName} "${text.slice(0, 60)}"`;
  });
}

/**
 * Whether no point of a 5 × 5 grid over the part of the element inside the
 * viewport shows the element on top; a stop taller or wider than the
 * viewport is sampled where it shows. Runs in the page.
 */
function isEntirelyCovered(element: Element): boolean {
  const samples = 5;
  const box = element.getBoundingClientRect();
  const left = Math.max(0, box.left);
  const top = Math.max(0, box.top);
  const width = Math.min(window.innerWidth, box.right) - left;
  const height = Math.min(window.innerHeight, box.bottom) - top;
  const root = element.getRootNode();
  const scope = root instanceof ShadowRoot ? root : document;
  if (width <= 0 || height <= 0) {
    return true;
  }
  for (let column = 0; column < samples; column += 1) {
    for (let row = 0; row < samples; row += 1) {
      const x = left + (width * (column + 0.5)) / samples;
      const y = top + (height * (row + 0.5)) / samples;
      const shown = scope.elementsFromPoint(x, y)[0];
      if (shown !== undefined && element.contains(shown)) {
        return false;
      }
    }
  }
  return true;
}

/** The element's box grown by `margin` on each side, inside the viewport, in viewport coordinates. Runs in the page. */
function focusArea(
  element: Element,
  margin: number,
): { x: number; y: number; width: number; height: number } {
  const box = element.getBoundingClientRect();
  const x = Math.max(0, box.left - margin);
  const y = Math.max(0, box.top - margin);
  const right = Math.min(window.innerWidth, box.right + margin);
  const bottom = Math.min(window.innerHeight, box.bottom + margin);
  return { x, y, width: Math.max(0, right - x), height: Math.max(0, bottom - y) };
}

/** Ends each of the page's finite animations and transitions, so a screenshot shows their end state. Runs in the page. */
function finishAnimations(): void {
  for (const animation of document.getAnimations()) {
    if (animation.effect?.getComputedTiming().endTime !== Infinity) {
      animation.finish();
    }
  }
}

/** Makes the element's text caret transparent and gives its previous `caret-color`. Runs in the page. */
function hideCaret(element: Element): string {
  if (!(element instanceof HTMLElement)) {
    return "";
  }
  const previous = element.style.caretColor;
  element.style.caretColor = "transparent";
  return previous;
}

/** Runs in the page. */
function showCaret(element: Element, previous: string): void {
  if (element instanceof HTMLElement) {
    element.style.caretColor = previous;
  }
}

/** Runs in the page. */
function blur(element: Element): void {
  if (element instanceof HTMLElement || element instanceof SVGElement) {
    element.blur();
  }
}

/** Runs in the page. */
function focus(element: Element): void {
  if (element instanceof HTMLElement || element instanceof SVGElement) {
    element.focus({ preventScroll: true });
  }
}
