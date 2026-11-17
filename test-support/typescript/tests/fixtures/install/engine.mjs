// A fixture platform package's engine, standing for the executable a
// Product's platform package carries: its version.
import manifest from "./package.json" with { type: "json" };

export const version = manifest.version;
