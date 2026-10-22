// The project sets `dark` and `reduce` (vitest.config.ts beside it).
import { expect, test } from "vitest";
import { browserState, browserStateReset } from "../../../src/vitest-browser.ts";

function media(): { readonly dark: boolean; readonly reduce: boolean } {
  return {
    dark: matchMedia("(prefers-color-scheme: dark)").matches,
    reduce: matchMedia("(prefers-reduced-motion: reduce)").matches,
  };
}

test("colour scheme and motion", async () => {
  await browserState({ colorScheme: "light", reducedMotion: "no-preference" });
  const set = media();

  await browserStateReset();

  expect([set, media()]).toEqual([
    { dark: false, reduce: false },
    { dark: true, reduce: true },
  ]);
});
