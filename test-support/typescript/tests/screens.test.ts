// Seam B (spec #326): `testEachScreen` gives each `screen:` Feature of a
// Feature inventory one named served case, which walks the screen with
// `keyboardWalk` and `focusChecks` and ends with the axe fixture's pass, on
// the executable of the screen's Product (ADRs 0073 and 0094). Playwright
// runs the repository's own `playwright.config.ts`, copied unchanged into a
// folder of its own with a fixture inventory, a fixture `console/screens.toml`
// and a spec file that calls `testEachScreen` on that folder. The stand-in
// Console serves each fixture screen only to the Product's Daemon, as its
// `hello` answers, and only in the screen's sign-in state.
import { copyFileSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join } from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, onTestFinished } from "vitest";
import { test, type TestSupportFixtures } from "@goodfetch/test-support";
import type { StandInScreen } from "./console-stand-in.ts";

const ROOT = join(import.meta.dirname, "..", "..", "..");
const FIXTURES = pathToFileURL(join(import.meta.dirname, "..", "src", "playwright.ts")).href;
const SCREENS = pathToFileURL(join(import.meta.dirname, "..", "src", "screens.ts")).href;
const STAND_IN = pathToFileURL(join(import.meta.dirname, "console-stand-in.ts")).href;
const CLI = createRequire(import.meta.url).resolve("@playwright/test/cli");

/** One fixture screen: its inventory entry, its `console/screens.toml` entry, and what the stand-in serves. */
interface FixtureScreen {
  readonly feature: string;
  readonly products: readonly ("collie" | "foxhound")[];
  /** Its `[[screen]]` entry's lines after `feature`, or `undefined` for no entry. */
  readonly setup: string | undefined;
  /** What the stand-in Console serves at the screen's route, if anything. */
  readonly served: { readonly route: string; readonly screen: StandInScreen } | undefined;
}

/** A case of the run, from Playwright's JSON report. */
interface Case {
  readonly title: string;
  readonly status: string;
  readonly error: string;
}

/** Runs `testEachScreen` on a fixture inventory of `screens` and gives each case Playwright reported. */
async function runScreens(
  processes: TestSupportFixtures["processes"],
  screens: readonly FixtureScreen[],
  orphans: readonly string[] = [],
  consoleFiles: Readonly<Record<string, string>> = {},
): Promise<Case[]> {
  // Inside the repository, so the copied configuration finds `@playwright/test`.
  mkdirSync(join(ROOT, "target"), { recursive: true });
  const project = mkdtempSync(join(ROOT, "target", "screens-test-"));
  onTestFinished(() => rmSync(project, { recursive: true, force: true }));
  copyFileSync(join(ROOT, "playwright.config.ts"), join(project, "playwright.config.ts"));
  mkdirSync(join(project, "features"));
  const features = screens.map(({ feature, products }) => ({
    id: feature,
    kind: "screen",
    area: null,
    milestone: null,
    status: "active",
    products,
  }));
  writeFileSync(join(project, "features", "inventory.json"), JSON.stringify({ features }));
  mkdirSync(join(project, "console"));
  for (const [name, text] of Object.entries(consoleFiles)) {
    mkdirSync(join(project, "console", name, ".."), { recursive: true });
    writeFileSync(join(project, "console", name), text);
  }
  const setups = screens
    .filter((screen) => screen.setup !== undefined)
    .map(({ feature, setup }) => `[[screen]]\nfeature = ${JSON.stringify(feature)}\n${setup}\n`)
    .concat(orphans.map((feature) => `[[screen]]\nfeature = ${JSON.stringify(feature)}\n`));
  writeFileSync(join(project, "console", "screens.toml"), setups.join("\n"));
  const served = Object.fromEntries(
    screens.flatMap(({ served: one }) => (one === undefined ? [] : [[one.route, one.screen]])),
  );
  const spec = `import { test } from ${JSON.stringify(FIXTURES)};
import { testEachScreen } from ${JSON.stringify(SCREENS)};
import { standInConsole } from ${JSON.stringify(STAND_IN)};

test.use({ consoleServer: standInConsole({ screens: ${JSON.stringify(served)} }) });

testEachScreen(import.meta.dirname);
`;
  writeFileSync(join(project, "screens.spec.ts"), spec);
  const report = join(project, "report.json");
  const env: NodeJS.ProcessEnv = {
    ...process.env,
    GOODFETCH_TEST_SUITE: "pre-merge",
    PLAYWRIGHT_JSON_OUTPUT_FILE: report,
  };
  delete env["PLAYWRIGHT_JUNIT_OUTPUT_FILE"];

  const run = processes.spawn(
    process.execPath,
    [CLI, "test", "--config", "playwright.config.ts", "--reporter", "json"],
    { env, cwd: project },
  );

  await run.exit;
  return casesOf(JSON.parse(readFileSync(report, "utf8")));
}

/** Each case of a Playwright JSON report, by its title, sorted. */
function casesOf(report: unknown): Case[] {
  const cases: Case[] = [];
  const visit = (suite: unknown): void => {
    for (const spec of arrayOf(suite, "specs")) {
      for (const run of arrayOf(spec, "tests")) {
        const result = arrayOf(run, "results")[0];
        cases.push({
          title: String(field(spec, "title")),
          status: String(field(result, "status")),
          error: arrayOf(result, "errors")
            .map((error) => String(field(error, "message")))
            .join("\n"),
        });
      }
    }
    arrayOf(suite, "suites").forEach(visit);
  };
  visit(report);
  return cases.toSorted((left, right) => left.title.localeCompare(right.title));
}

function field(value: unknown, key: string): unknown {
  return typeof value === "object" && value !== null ? Reflect.get(value, key) : undefined;
}

function arrayOf(value: unknown, key: string): unknown[] {
  const found = field(value, key);
  return Array.isArray(found) ? found : [];
}

/** The walk of the `keyboard-clean` fixture page, as `console/screens.toml` records it. */
const CLEAN_WALK =
  'walk = ["link Docs", "button Actions", "button Settings", "textbox Email", "button Save"]';

/** An element the `keyboard-clean` fixture page shows from the start, as its ready element. */
const CLEAN_READY = 'ready = "button Save"';

describe("testEachScreen", () => {
  test("gives two screens two named cases, each on its Product's executable in its sign-in state", async ({
    processes,
  }) => {
    const cases = await runScreens(processes, [
      {
        feature: "screen:/runs",
        products: ["collie", "foxhound"],
        setup: `route = "/runs"\nsign_in = "signed-in"\n${CLEAN_WALK}\n${CLEAN_READY}`,
        served: {
          route: "/runs",
          screen: { page: "keyboard-clean", product: "collie", signedIn: true },
        },
      },
      {
        feature: "screen:/crawls/$crawl",
        products: ["foxhound"],
        setup: `route = "/crawls/c-1"\nsign_in = "signed-out"\n${CLEAN_WALK}\n${CLEAN_READY}`,
        served: {
          route: "/crawls/c-1",
          screen: { page: "keyboard-clean", product: "foxhound", signedIn: false },
        },
      },
    ]);

    expect(cases).toEqual([
      { title: "screen:/crawls/$crawl", status: "passed", error: "" },
      { title: "screen:/runs", status: "passed", error: "" },
    ]);
  });

  test("fails the case of a screen with a focus trap, an axe violation, or no route or sign-in state, each by its name", async ({
    processes,
  }) => {
    const cases = await runScreens(processes, [
      {
        feature: "screen:/trap",
        products: ["collie", "foxhound"],
        setup: `route = "/trap"\nsign_in = "signed-out"\n${CLEAN_WALK}\nready = "button First"`,
        served: {
          route: "/trap",
          screen: { page: "keyboard-trap", product: "collie", signedIn: false },
        },
      },
      {
        feature: "screen:/inaccessible",
        products: ["collie", "foxhound"],
        setup:
          'route = "/inaccessible"\nsign_in = "signed-out"\nwalk = ["link Top"]\nready = "link Top"',
        served: {
          route: "/inaccessible",
          screen: { page: "inaccessible", product: "collie", signedIn: false },
        },
      },
      {
        feature: "screen:/no-route",
        products: ["collie", "foxhound"],
        setup: `sign_in = "signed-in"\n${CLEAN_WALK}\n${CLEAN_READY}`,
        served: undefined,
      },
      {
        feature: "screen:/no-sign-in",
        products: ["collie", "foxhound"],
        setup: `route = "/no-sign-in"\n${CLEAN_WALK}\n${CLEAN_READY}`,
        served: undefined,
      },
      {
        feature: "screen:/no-ready",
        products: ["collie", "foxhound"],
        setup: `route = "/no-ready"\nsign_in = "signed-in"\n${CLEAN_WALK}`,
        served: undefined,
      },
      {
        feature: "screen:/no-entry",
        products: ["foxhound"],
        setup: undefined,
        served: undefined,
      },
    ]);

    expect(cases.map(({ title, status }) => ({ title, status }))).toEqual([
      { title: "screen:/inaccessible", status: "failed" },
      { title: "screen:/no-entry", status: "failed" },
      { title: "screen:/no-ready", status: "failed" },
      { title: "screen:/no-route", status: "failed" },
      { title: "screen:/no-sign-in", status: "failed" },
      { title: "screen:/trap", status: "failed" },
    ]);
    const errors = Object.fromEntries(cases.map(({ title, error }) => [title, error]));
    // A test support error's name carries its code: `TestSupportError(<code>)`.
    expect(errors["screen:/trap"]).toContain("(focus_trapped)");
    expect(errors["screen:/inaccessible"]).toContain("(axe_violations)");
    for (const [screen, missing] of [
      ["screen:/no-route", "no route"],
      ["screen:/no-sign-in", "no sign-in state"],
      ["screen:/no-ready", "no ready element"],
      ["screen:/no-entry", "no route, no sign-in state, no keyboard walk and no ready element"],
    ] as const) {
      expect(errors[screen]).toContain("(screen_setup_missing)");
      expect(errors[screen]).toContain(`${screen} has ${missing}`);
    }
  });

  test("fails the case of a screen whose walk is not the one recorded, or has no stop", async ({
    processes,
  }) => {
    const cases = await runScreens(processes, [
      {
        feature: "screen:/moved",
        products: ["collie", "foxhound"],
        setup:
          'route = "/moved"\nsign_in = "signed-out"\nwalk = ["button Actions", "link Docs", "button Settings", "textbox Email", "button Save"]\nready = "button Save"',
        served: {
          route: "/moved",
          screen: { page: "keyboard-clean", product: "collie", signedIn: false },
        },
      },
      {
        feature: "screen:/empty",
        products: ["collie", "foxhound"],
        setup: 'route = "/empty"\nsign_in = "signed-out"\nwalk = []\nready = "heading Accessible"',
        served: {
          route: "/empty",
          screen: { page: "accessible", product: "collie", signedIn: false },
        },
      },
    ]);

    const errors = Object.fromEntries(cases.map(({ title, error }) => [title, error]));
    expect(cases.map(({ title, status }) => ({ title, status }))).toEqual([
      { title: "screen:/empty", status: "failed" },
      { title: "screen:/moved", status: "failed" },
    ]);
    expect(errors["screen:/moved"]).toContain("(keyboard_walk_changed)");
    expect(errors["screen:/empty"]).toContain("(keyboard_walk_empty)");
  });

  test("fails the case of a screen that scrolls sideways at 320 CSS pixels, or keeps an animation running under reduced motion", async ({
    processes,
  }) => {
    const walk = 'walk = ["button Refresh"]\nready = "button Refresh"';
    const cases = await runScreens(processes, [
      {
        feature: "screen:/wide",
        products: ["collie", "foxhound"],
        setup: `route = "/wide"\nsign_in = "signed-out"\n${walk}`,
        served: {
          route: "/wide",
          screen: { page: "screen-wide", product: "collie", signedIn: false },
        },
      },
      {
        feature: "screen:/spinning",
        products: ["collie", "foxhound"],
        setup: `route = "/spinning"\nsign_in = "signed-out"\n${walk}`,
        served: {
          route: "/spinning",
          screen: { page: "screen-spinning", product: "collie", signedIn: false },
        },
      },
    ]);

    const errors = Object.fromEntries(cases.map(({ title, error }) => [title, error]));
    expect(cases.map(({ title, status }) => ({ title, status }))).toEqual([
      { title: "screen:/spinning", status: "failed" },
      { title: "screen:/wide", status: "failed" },
    ]);
    expect(errors["screen:/wide"]).toContain("(horizontal_scroll)");
    expect(errors["screen:/spinning"]).toContain("(animations_running)");
  });

  test("walks a screen once it shows the element its setup names as ready, by its role and name or by its text", async ({
    processes,
  }) => {
    const cases = await runScreens(processes, [
      {
        feature: "screen:/late",
        products: ["collie", "foxhound"],
        setup:
          'route = "/late"\nsign_in = "signed-out"\nready = "button Late"\nwalk = ["button Early", "button Late"]',
        served: {
          route: "/late",
          screen: { page: "screen-late", product: "collie", signedIn: false },
        },
      },
      {
        feature: "screen:/late-text",
        products: ["collie", "foxhound"],
        setup:
          'route = "/late-text"\nsign_in = "signed-out"\nready = "text Late"\nwalk = ["button Early", "button Late"]',
        served: {
          route: "/late-text",
          screen: { page: "screen-late", product: "collie", signedIn: false },
        },
      },
    ]);

    expect(cases).toEqual([
      { title: "screen:/late", status: "passed", error: "" },
      { title: "screen:/late-text", status: "passed", error: "" },
    ]);
  });

  test("fails by name a setup whose screen the inventory does not list", async ({ processes }) => {
    const cases = await runScreens(processes, [], ["screen:/gone"]);

    expect(cases.map(({ title, status }) => ({ title, status }))).toEqual([
      { title: "screen:/gone", status: "failed" },
    ]);
    expect(cases[0]?.error).toContain("(screen_setup_orphaned)");
  });

  test("fails when the Console has routes and the inventory names no screen", async ({
    processes,
  }) => {
    const cases = await runScreens(processes, [], [], {
      "collie/routes.json": JSON.stringify({ routes: ["/runs"] }),
    });

    expect(cases.map(({ title, status }) => ({ title, status }))).toEqual([
      { title: "every screen", status: "failed" },
    ]);
    expect(cases[0]?.error).toContain("(screens_missing)");
  });
});
