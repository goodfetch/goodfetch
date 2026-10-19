// The reflow check of ADR 0073 (WCAG 1.4.10): at 320 × 256 CSS pixels a
// page needs no horizontal scroll, except inside data tables and code
// blocks, which scroll in their own container.
import type { Page } from "@playwright/test";
import { TestSupportError } from "./errors.ts";

/** The viewport WCAG's reflow criterion names: 320 CSS pixels wide, 256 high. */
const REFLOW_VIEWPORT = { width: 320, height: 256 } as const;

/**
 * Lays `page` out at 320 × 256 CSS pixels and fails with
 * `horizontal_scroll` when the page scrolls sideways, or an element of it
 * does that is not a data table or code block (a `table`, an element whose
 * role is `table`, `grid` or `treegrid`, a `pre` or a `code`) or a scroll
 * container whose only child is one. The page gets its viewport back
 * afterwards.
 */
export async function reflow(page: Page): Promise<void> {
  const viewport = page.viewportSize();
  await page.setViewportSize(REFLOW_VIEWPORT);
  try {
    const scrolling = await page.evaluate(sidewaysScroll);
    if (scrolling.length > 0) {
      throw new TestSupportError(
        "horizontal_scroll",
        `At ${REFLOW_VIEWPORT.width} × ${REFLOW_VIEWPORT.height} CSS pixels, ${page.url()} scrolls sideways outside a data table or code block:\n${scrolling.map((line) => `- ${line}`).join("\n")}`,
      );
    }
  } finally {
    if (viewport !== null) {
      await page.setViewportSize(viewport);
    }
  }
}

/**
 * Runs in the page: after two frames, so that the page's resize handlers
 * have run, describes the page's sideways scroll and each element's that is
 * not a table or code block's.
 */
async function sidewaysScroll(): Promise<string[]> {
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const tableOrCode = 'table, [role="table"], [role="grid"], [role="treegrid"], pre, code';
  // oxlint-disable-next-line unicorn/consistent-function-scoping -- `page.evaluate` sends this function's source alone, so what it calls must be inside it.
  const describe = (element: Element): string => {
    const id = element.id === "" ? "" : `#${element.id}`;
    const name = element.getAttribute("aria-label") ?? element.textContent?.trim().slice(0, 40);
    return `<${element.localName}${id}>${name === undefined || name === "" ? "" : ` "${name}"`}`;
  };
  const found: string[] = [];
  const root = document.documentElement;
  if (root.scrollWidth > root.clientWidth) {
    const beyond = [...document.body.querySelectorAll("*")].filter((element) => {
      const parent = element.parentElement;
      return (
        element.getBoundingClientRect().right > root.clientWidth &&
        (parent === null || parent.getBoundingClientRect().right <= root.clientWidth)
      );
    });
    found.push(
      `the page is ${root.scrollWidth} CSS pixels wide${beyond.length === 0 ? "" : `, past its right edge: ${beyond.slice(0, 5).map(describe).join(", ")}`}`,
    );
  }
  for (const element of document.body.querySelectorAll("*")) {
    const overflow = getComputedStyle(element).overflowX;
    const scrolls =
      (overflow === "auto" || overflow === "scroll") && element.scrollWidth > element.clientWidth;
    const only = element.children.length === 1 ? element.children[0] : undefined;
    const exempt = element.matches(tableOrCode) || (only?.matches(tableOrCode) ?? false);
    if (scrolls && !exempt) {
      found.push(`${describe(element)} scrolls sideways`);
    }
  }
  return found;
}
