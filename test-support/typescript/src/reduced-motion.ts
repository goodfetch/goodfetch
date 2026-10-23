// The reduced-motion check of ADR 0073: under `reducedMotion: 'reduce'`,
// `document.getAnimations()` holds no running animation after an action.
import type { Page } from "@playwright/test";
import { TestSupportError } from "./errors.ts";

/**
 * Fails with `animations_running` when an animation or transition of
 * `page`'s document is still running two frames after the call, which a
 * test makes right after the action it checks. It fails with
 * `reduced_motion_off` on a page that does not prefer reduced motion: set
 * `reducedMotion: 'reduce'` in the project, with `test.use` or through
 * `page.emulateMedia`.
 */
export async function noRunningAnimations(page: Page): Promise<void> {
  const reduced = await page.evaluate(() => matchMedia("(prefers-reduced-motion: reduce)").matches);
  if (!reduced) {
    throw new TestSupportError(
      "reduced_motion_off",
      `${page.url()} does not prefer reduced motion: set reducedMotion: 'reduce' before checking its animations`,
    );
  }
  const running = await page.evaluate(runningAnimations);
  if (running.length > 0) {
    throw new TestSupportError(
      "animations_running",
      `${page.url()} still runs animations under prefers-reduced-motion: reduce:\n${running.map((line) => `- ${line}`).join("\n")}`,
    );
  }
}

/** Runs in the page: after two frames, describes each running animation and its element. */
async function runningAnimations(): Promise<string[]> {
  await new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  return document
    .getAnimations()
    .filter((animation) => animation.playState === "running")
    .map((animation) => {
      const what =
        animation instanceof CSSAnimation
          ? `CSS animation "${animation.animationName}"`
          : animation instanceof CSSTransition
            ? `CSS transition of ${animation.transitionProperty}`
            : `script animation${animation.id === "" ? "" : ` "${animation.id}"`}`;
      const target = animation.effect instanceof KeyframeEffect ? animation.effect.target : null;
      const on =
        target === null
          ? ""
          : ` on <${target.localName}${target.id === "" ? "" : `#${target.id}`}>`;
      return `${what}${on}`;
    });
}
