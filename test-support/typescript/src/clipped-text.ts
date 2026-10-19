// The clipped-text check of ADR 0073 (WCAG 1.4.4 and 1.4.12): no text is
// cut off by an `overflow` box without a way to read it in full. It runs in
// a Vitest Browser Mode test, usually under `textSpacing()`.
import type { Locator } from "vitest/browser";
import { TestSupportError } from "./errors.ts";

/** How far, in CSS pixels, text may pass its box's edge before it counts as clipped (subpixel layout). */
const TOLERANCE = 0.5;

/**
 * Fails with `text_clipped` when text inside the element `locator` finds is
 * cut off by an ancestor whose `overflow` hides or clips it, such as a
 * truncated label or a fixed-height box under larger text. Text an ancestor
 * lets the reader scroll to (`overflow: auto` or `scroll`) can be read in
 * full, so it passes; so does text that is not rendered or visible, and
 * visually hidden text (in a box at most 1 CSS pixel wide or high), which
 * only assistive technology reads.
 */
export async function clippedText(locator: Locator): Promise<void> {
  const root = locator.element();
  const clipped: string[] = [];
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node !== null; node = walker.nextNode()) {
    if (node instanceof Text) {
      const clipper = clippingAncestor(node);
      if (clipper !== undefined) {
        clipped.push(`"${node.data.trim().slice(0, 60)}", cut off by <${clipper.localName}>`);
      }
    }
  }
  if (clipped.length > 0) {
    throw new TestSupportError(
      "text_clipped",
      `Text is cut off with no way to read it in full:\n${clipped.map((line) => `- ${line}`).join("\n")}`,
    );
  }
}

/** The ancestor that cuts `text` off on an axis it does not let the reader scroll, if any. */
function clippingAncestor(text: Text): Element | undefined {
  const parent = text.parentElement;
  if (
    text.data.trim() === "" ||
    parent === null ||
    !parent.checkVisibility({ visibilityProperty: true })
  ) {
    return undefined;
  }
  const range = document.createRange();
  range.selectNodeContents(text);
  const rendered = range.getBoundingClientRect();
  if (rendered.width === 0 && rendered.height === 0) {
    return undefined;
  }
  // What must stay readable: the text, and past a scroll container on an
  // axis, the part of that container's box the text spans on it.
  let { left, right, top, bottom } = rendered;
  // The root's and the body's overflow apply to the viewport, which scrolls.
  for (
    let ancestor: Element | null = parent;
    ancestor !== null && ancestor !== document.body && ancestor !== document.documentElement;
    ancestor = ancestor.parentElement
  ) {
    const style = getComputedStyle(ancestor);
    const hidesX = hides(style.overflowX);
    const hidesY = hides(style.overflowY);
    const scrollsX = scrolls(style.overflowX);
    const scrollsY = scrolls(style.overflowY);
    if (!hidesX && !hidesY && !scrollsX && !scrollsY) {
      continue;
    }
    if ((hidesX || hidesY) && (ancestor.clientWidth <= 1 || ancestor.clientHeight <= 1)) {
      return undefined;
    }
    const edges = ancestor.getBoundingClientRect();
    const boxLeft = edges.left + ancestor.clientLeft;
    const boxRight = boxLeft + ancestor.clientWidth;
    const boxTop = edges.top + ancestor.clientTop;
    const boxBottom = boxTop + ancestor.clientHeight;
    const cutX = left < boxLeft - TOLERANCE || right > boxRight + TOLERANCE;
    const cutY = top < boxTop - TOLERANCE || bottom > boxBottom + TOLERANCE;
    if ((hidesX && cutX) || (hidesY && cutY)) {
      return ancestor;
    }
    if (scrollsX) {
      left = Math.max(left, boxLeft);
      right = Math.min(right, boxRight);
    }
    if (scrollsY) {
      top = Math.max(top, boxTop);
      bottom = Math.min(bottom, boxBottom);
    }
  }
  return undefined;
}

function hides(overflow: string): boolean {
  return overflow === "hidden" || overflow === "clip";
}

function scrolls(overflow: string): boolean {
  return overflow === "auto" || overflow === "scroll";
}
