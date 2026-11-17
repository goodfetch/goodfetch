// A fixture of `@goodfetch/collie-client`, which both fixture Products pin
// exactly (ADR 0095): its version, and the `engine_not_found` error a
// Product's bin writes when its platform package is missing.
import manifest from "./package.json" with { type: "json" };

export const version = manifest.version;

export function engineNotFound(platformPackage, cause) {
  return {
    code: "engine_not_found",
    group: "unavailable",
    retryable: false,
    message: `the platform package ${platformPackage} is not installed: ${String(cause)}`,
    fix: `Install the package again without omitting optional dependencies, or set the engine-path variable.`,
  };
}
