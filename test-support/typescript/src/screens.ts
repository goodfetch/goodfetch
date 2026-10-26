// The keyboard walk of every Console screen (ADR 0073): one named served
// case per `screen:` Feature of the inventory, so a new screen gets its
// keyboard walk and its axe pass with no test written for it. Each case
// opens its screen on its own Daemon, run by the executable of the screen's
// Product (Collie's for a screen both Products carry, Foxhound's for
// Foxhound's screens, ADR 0094), at the route and in the sign-in state area
// 12 gives it in `console/screens.toml`, runs `keyboardWalk`, whose stops
// must be the walk recorded there (`keyboard_walk_changed`), and never none
// (`keyboard_walk_empty`), then `focusChecks`, then `reflow` (no sideways
// scroll at 320 × 256 CSS pixels) and, since every case prefers reduced
// motion, `noRunningAnimations` after the walk's actions, and ends with the
// axe fixture's pass, whose `wcag22aa` tag checks target size.
//
// A screen without a route, a sign-in state or a walk fails its case by name
// with `screen_setup_missing`; it is never skipped. A setup whose screen the
// inventory does not list fails by its Feature (`screen_setup_orphaned`),
// and a Console with routes whose inventory names no screen fails as a case
// of its own (`screens_missing`), so no screen drops out unseen. The cases declare no Feature:
// they check how a screen is built, not the outcome its Feature names.
import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import type { Page } from "@playwright/test";
import { TestSupportError } from "./errors.ts";
import { readScreens, type Screen } from "./features.ts";
import { focusChecks, keyboardWalk } from "./focus.ts";
import { test } from "./playwright.ts";
import { noRunningAnimations } from "./reduced-motion.ts";
import { reflow } from "./reflow.ts";
import { readScreenSetups, SCREEN_SETUPS_FILE, type ScreenSetup } from "./screen-setups.ts";

export {
  type ReadyElement,
  parseScreenSetups,
  readScreenSetups,
  SCREEN_SETUPS_FILE,
  type ScreenSetup,
  type SignInState,
} from "./screen-setups.ts";

/**
 * Declares one served case per `screen:` Feature of the inventory of the
 * workspace at `root`, named by the Feature, with the screen's setup from
 * that workspace's `console/screens.toml`.
 */
export function testEachScreen(root: string): void {
  const setups = readScreenSetups(root);
  const screens = readScreens(root);
  for (const screen of screens) {
    test.describe(() => {
      test.use({ product: screen.product, reducedMotion: "reduce" });
      caseOf(screen, setups.get(screen.feature));
    });
  }
  const listed = new Set(screens.map(({ feature }) => feature));
  for (const feature of setups.keys()) {
    if (!listed.has(feature)) {
      test(feature, () => {
        throw new TestSupportError(
          "screen_setup_orphaned",
          `${SCREEN_SETUPS_FILE} sets up ${feature}, which is no active \`screen:\` Feature of the inventory: remove its entry, or run \`just codegen\``,
        );
      });
    }
  }
  const routeLists = consoleRouteLists(root);
  if (screens.length === 0 && routeLists.length > 0) {
    test("every screen", () => {
      throw new TestSupportError(
        "screens_missing",
        `the Console has routes (${routeLists.join(", ")}), and the inventory names no \`screen:\` Feature: run \`just codegen\``,
      );
    });
  }
}

/** The route lists the Console's packages commit, `console/<package>/routes.json`, from which `screen:` Features come. */
function consoleRouteLists(root: string): string[] {
  const folder = join(root, "console");
  if (!existsSync(folder)) {
    return [];
  }
  return readdirSync(folder, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => join("console", entry.name, "routes.json"))
    .filter((path) => existsSync(join(root, path)));
}

function caseOf(screen: Screen, setup: ScreenSetup | undefined): void {
  const route = setup?.route;
  const signInState = setup?.signIn;
  const walk = setup?.walk;
  if (route === undefined || signInState === undefined || walk === undefined) {
    const missing = [
      ...(route === undefined ? ["no route"] : []),
      ...(signInState === undefined ? ["no sign-in state"] : []),
      ...(walk === undefined ? ["no keyboard walk"] : []),
    ];
    const listed =
      missing.length === 1
        ? missing[0]
        : `${missing.slice(0, -1).join(", ")} and ${missing.at(-1) ?? ""}`;
    test(screen.feature, () => {
      throw new TestSupportError(
        "screen_setup_missing",
        `${screen.feature} has ${listed} in ${SCREEN_SETUPS_FILE}, where area 12 gives each screen all three`,
      );
    });
    return;
  }
  test(screen.feature, async ({ page, daemon, signIn }, testInfo) => {
    if (signInState === "signed-in") {
      await signIn.withLink(page);
    }
    const response = await page.goto(new URL(route, daemon.url).href);
    if (response === null || !response.ok()) {
      const answer = response === null ? "nothing" : `HTTP ${response.status()}`;
      throw new TestSupportError(
        "screen_not_served",
        `${screen.feature}'s route ${route} answered ${answer} on ${daemon.product}'s executable, ${signInState}`,
      );
    }
    const ready = setup?.ready;
    if (ready !== undefined) {
      await test.step(`ready: ${ready.role} ${ready.name}`, () =>
        page
          // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- the setup names an ARIA role; a misspelt one finds nothing, and the wait fails naming it.
          .getByRole(ready.role as Parameters<Page["getByRole"]>[0], {
            name: ready.name,
            exact: true,
          })
          .waitFor({ state: "visible" }));
    }
    const stops = await test.step("keyboardWalk", () => keyboardWalk(page));
    await testInfo.attach("keyboard-walk", {
      body: JSON.stringify(stops, null, 2),
      contentType: "application/json",
    });
    if (stops.length === 0) {
      throw new TestSupportError(
        "keyboard_walk_empty",
        `${screen.feature} has no stop for the keyboard: Tab reaches nothing on it`,
      );
    }
    const walked = stops.map(({ role, name }) => `${role} ${name}`);
    if (JSON.stringify(walked) !== JSON.stringify(walk)) {
      throw new TestSupportError(
        "keyboard_walk_changed",
        `${screen.feature}'s keyboard walk is ${JSON.stringify(walked)}, not the ${JSON.stringify(walk)} ${SCREEN_SETUPS_FILE} records; fix the screen, or record the new walk`,
      );
    }
    await test.step("focusChecks", () => focusChecks(page));
    await test.step("reflow", () => reflow(page));
    await test.step("noRunningAnimations", () => noRunningAnimations(page));
    // The axe fixture's pass ends the case, once it has passed.
  });
}
