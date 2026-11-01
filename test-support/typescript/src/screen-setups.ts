// The screens' setup file, `console/screens.toml` (ADR 0073): how each
// Console screen's generated keyboard walk opens it, one `[[screen]]` entry
// each, naming the screen's Feature, the route it opens at with every
// parameter filled, whether a person is signed in, and its keyboard walk:
// each stop's role and accessible name, in Tab order, and, for a screen
// that shows what it reads from the engine, the element it shows once it
// has, which the walk waits for: by its role and accessible name, or by
// the text it shows (`text <text>`) when that answer has no role with a
// name. Area 12 fills it;
// `testEachScreen` reads it. A field not given yet is `undefined` here, so
// that screen's case fails by name rather than the whole file.
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { parse } from "smol-toml";
import { TestSupportError } from "./errors.ts";

/** The screens' setup file, from the workspace's folder. */
export const SCREEN_SETUPS_FILE: string = join("console", "screens.toml");

/** Whether a person is signed in to the Console when a screen opens. */
export type SignInState = "signed-in" | "signed-out";

const SIGN_IN_STATES: readonly SignInState[] = ["signed-in", "signed-out"];

/** How a screen's case opens it; a field area 12 has not given yet is `undefined`. */
export interface ScreenSetup {
  /** The screen's Feature, `screen:<route>`. */
  readonly feature: string;
  /** The path it opens at, from the Console's URL, with every parameter filled: `/runs/r-1`. */
  readonly route: string | undefined;
  /** `signed-in` follows the executable's single-user sign-in link before the screen opens. */
  readonly signIn: SignInState | undefined;
  /** Its keyboard walk, each stop as `<role> <accessible name>`, in Tab order: `link Runs`. */
  readonly walk: readonly string[] | undefined;
  /** The element the screen shows once it has read what it shows, which the case waits for before its walk. */
  readonly ready: ReadyElement | undefined;
}

/**
 * An element by its role and accessible name, written `<role> <accessible
 * name>` as a walk's stop is, or by the text it shows, written `text
 * <text>`, for an answer with no role that takes a name (a paragraph, a
 * status message).
 */
export type ReadyElement =
  | { readonly role: string; readonly name: string }
  | { readonly text: string };

/** The pseudo-role of a ready element named by its text: no ARIA role is `text`. */
const READY_TEXT = "text";

const FIELDS: readonly string[] = ["feature", "route", "sign_in", "walk", "ready"];

/**
 * The entries of a screen setup file's text, by Feature; fails with
 * `screen_setups_invalid` on an entry without a `screen:` Feature, one
 * repeating another's Feature, or one with a field it does not define or
 * of the wrong type.
 */
export function parseScreenSetups(text: string): ReadonlyMap<string, ScreenSetup> {
  let document: Record<string, unknown>;
  try {
    document = parse(text);
  } catch (error) {
    throw new TestSupportError("screen_setups_invalid", "the screen setup file is no TOML", {
      cause: error,
    });
  }
  const unknownKeys = Object.keys(document).filter((key) => key !== "screen");
  if (unknownKeys.length > 0) {
    throw new TestSupportError(
      "screen_setups_invalid",
      `the screen setup file has keys it does not define: ${unknownKeys.join(", ")}`,
    );
  }
  const entries = document["screen"] ?? [];
  if (!Array.isArray(entries)) {
    throw new TestSupportError(
      "screen_setups_invalid",
      "the screen setup file's `screen` is no array of tables",
    );
  }
  const setups = new Map<string, ScreenSetup>();
  entries.forEach((entry: unknown, index) => {
    const setup = setupAt(entry, index + 1);
    if (setups.has(setup.feature)) {
      throw new TestSupportError(
        "screen_setups_invalid",
        `screen entry ${index + 1} repeats ${setup.feature}`,
      );
    }
    setups.set(setup.feature, setup);
  });
  return setups;
}

function setupAt(entry: unknown, position: number): ScreenSetup {
  const invalid = (why: string): TestSupportError =>
    new TestSupportError("screen_setups_invalid", `screen entry ${position} ${why}`);
  if (typeof entry !== "object" || entry === null || Array.isArray(entry)) {
    throw invalid("is no table");
  }
  const fields: Record<string, unknown> = { ...entry };
  const unknownFields = Object.keys(fields).filter((field) => !FIELDS.includes(field));
  if (unknownFields.length > 0) {
    throw invalid(`has fields it does not define: ${unknownFields.join(", ")}`);
  }
  const { feature, route, sign_in: signIn, walk, ready } = fields;
  if (typeof feature !== "string" || !feature.startsWith("screen:")) {
    throw invalid("names no `screen:` Feature");
  }
  if (route !== undefined && (typeof route !== "string" || !route.startsWith("/"))) {
    throw invalid(`gives ${feature} a route that is no path from the Console's URL`);
  }
  const state = SIGN_IN_STATES.find((known) => known === signIn);
  if (signIn !== undefined && state === undefined) {
    throw invalid(`gives ${feature} a sign-in state other than ${SIGN_IN_STATES.join(" or ")}`);
  }
  if (
    walk !== undefined &&
    (!Array.isArray(walk) || !walk.every((stop) => typeof stop === "string" && stop !== ""))
  ) {
    throw invalid(`gives ${feature} a walk that is no list of \`<role> <name>\` stops`);
  }
  const space = typeof ready === "string" ? ready.indexOf(" ") : -1;
  if (
    ready !== undefined &&
    (typeof ready !== "string" || space <= 0 || space === ready.length - 1)
  ) {
    throw invalid(
      `gives ${feature} a ready element that is no \`<role> <name>\` or \`${READY_TEXT} <text>\``,
    );
  }
  return { feature, route, signIn: state, walk, ready: readyElementOf(ready, space) };
}

/** The ready element `ready` names, its role (or `text`) before the space at `space`. */
function readyElementOf(ready: unknown, space: number): ReadyElement | undefined {
  if (typeof ready !== "string") {
    return undefined;
  }
  const role = ready.slice(0, space);
  const rest = ready.slice(space + 1);
  return role === READY_TEXT ? { text: rest } : { role, name: rest };
}

/** The entries of the screen setup file of the workspace at `root`; `screen_setups_invalid` when it cannot be read. */
export function readScreenSetups(root: string): ReadonlyMap<string, ScreenSetup> {
  const path = join(root, SCREEN_SETUPS_FILE);
  let text: string;
  try {
    text = readFileSync(path, "utf8");
  } catch (error) {
    throw new TestSupportError("screen_setups_invalid", `${path} cannot be read`, {
      cause: error,
    });
  }
  return parseScreenSetups(text);
}
