// The routes of Collie's build (ADR 0072; spec #592's `screens`): plain
// data, which the package commits as `routes.json` at its root for the
// Feature inventory's `screen:` Features, and at which Collie's executable
// serves the page. A screen module declares its own; this file lists them.
// A segment that begins with `$` stands for any one segment of the page's
// path, the route's parameter of that name.

/** "Needs you": everything waiting for a person, across every run (ADR 0031). */
export const NEEDS_YOU = "/needs-you";

/** Runs: every run of the user and major, filtered and sorted, its filter in the address's query (ADRs 0031 and 0117). */
export const RUNS = "/runs";

/** A run's page, at the address a Run report's `consoleUrl` gives (ADR 0102). */
export const RUN = "/runs/$run";

/** Engine & settings: every effective value with its layer and lock (ADR 0031). */
export const ENGINE_SETTINGS = "/settings";

/** Every route of Collie's build: its side menu's, in its order, then the others. */
export const ROUTES: readonly string[] = [NEEDS_YOU, RUNS, ENGINE_SETTINGS, RUN];

/** A route's parameters, by name. */
export type RouteParams = Readonly<Record<string, string>>;

/** The parameters `path` gives `route`, decoded; none when `path` is not one of its. */
export function paramsOf(route: string, path: string): RouteParams | undefined {
  const expected = route.split("/");
  const given = path.split("/");
  if (expected.length !== given.length) {
    return undefined;
  }
  const params: Record<string, string> = {};
  for (const [index, segment] of expected.entries()) {
    const value = given[index] ?? "";
    if (segment.startsWith("$")) {
      if (value === "") {
        return undefined;
      }
      params[segment.slice(1)] = decodeURIComponent(value);
    } else if (segment !== value) {
      return undefined;
    }
  }
  return params;
}
