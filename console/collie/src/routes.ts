// The routes of Collie's build (ADR 0072; spec #592's `screens`): plain
// data, which the package commits as `routes.json` at its root for the
// Feature inventory's `screen:` Features, and at which Collie's executable
// serves the page. A screen module declares its own; this file lists them.

/** "Needs you": everything waiting for a person, across every run (ADR 0031). */
export const NEEDS_YOU = "/needs-you";

/** Engine & settings: every effective value with its layer and lock (ADR 0031). */
export const ENGINE_SETTINGS = "/settings";

/** Every route of Collie's build, in its side menu's order. */
export const ROUTES: readonly string[] = [NEEDS_YOU, ENGINE_SETTINGS];
