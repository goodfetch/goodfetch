// The keyboard walk of every Console screen (ADR 0073; TESTING.md,
// Accessibility): one named served case per `screen:` Feature of
// `features/inventory.json`, which walks the screen with `keyboardWalk` and
// `focusChecks`, checks its reflow and its motion when reduced, and ends
// with the axe fixture's pass, on the executable of the screen's Product
// (ADR 0094). A new screen gets its case with no change here; area 12 gives
// it its route and sign-in state in `screens.toml`. Together the cases are
// what makes every screen meet WCAG 2.2 AA as far as a test can tell, so
// they declare that story (ADR 0073's note of 2026-10-26).
import { join } from "node:path";
import { test } from "@goodfetch/test-support/playwright";
import { testEachScreen } from "@goodfetch/test-support/screens";

test.describe(
  "every screen to WCAG 2.2 AA",
  { annotation: { type: "covers", description: "story:12-130" } },
  () => {
    testEachScreen(join(import.meta.dirname, ".."));
  },
);
