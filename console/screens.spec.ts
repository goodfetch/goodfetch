// The keyboard walk of every Console screen (ADR 0073; TESTING.md,
// Accessibility): one named served case per `screen:` Feature of
// `features/inventory.json`, which walks the screen with `keyboardWalk` and
// `focusChecks` and ends with the axe fixture's pass, on the executable of
// the screen's Product (ADR 0094). A new screen gets its case with no change
// here; area 12 gives it its route and sign-in state in `screens.toml`.
import { join } from "node:path";
import { testEachScreen } from "@goodfetch/test-support/screens";

testEachScreen(join(import.meta.dirname, ".."));
