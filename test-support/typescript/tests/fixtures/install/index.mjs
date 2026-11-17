// A fixture Product package's entry point: its version, which the smoke
// script checks.
import manifest from "./package.json" with { type: "json" };

export const version = manifest.version;
