// Where Collie's executable is (ADRs 0024, 0090 and 0094): the program's
// `GOODFETCH_COLLIE_ENGINE_PATH` when it sets one, else the executable of
// the platform package `@goodfetch/collie` installed. Foxhound's variable is
// another Product's and is never read here.
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import { EngineNotFoundError, SettingInvalidError } from "@goodfetch/collie-client";

/** The variable naming Collie's executable (ADR 0090). */
export const ENGINE_PATH_VARIABLE = "GOODFETCH_COLLIE_ENGINE_PATH";

/** The executable's name (ADR 0090). */
const EXECUTABLE = "goodfetch-collie";

/** Where to start Collie's engine, and what to do when it is not there. */
export interface EngineLocation {
  readonly path: string;
  /** The `fix` of `engine_not_found` when `path` is no executable. */
  readonly notFoundFix: string;
}

/**
 * The executable `env` names, or the platform package's; `engine_not_found`
 * when neither exists, and `setting_invalid` when the variable is set but
 * empty.
 */
export function locateEngine(env: Readonly<Record<string, string>>): EngineLocation {
  const named = env[ENGINE_PATH_VARIABLE];
  if (named === "") {
    // An empty value is never taken for unset (ADR 0083).
    throw new SettingInvalidError(`${ENGINE_PATH_VARIABLE} is set but empty`, {
      fix: `Set ${ENGINE_PATH_VARIABLE} to Collie's executable, ${EXECUTABLE}, or leave it unset to use the one @goodfetch/collie installed.`,
      details: { key: ENGINE_PATH_VARIABLE, reason: "an empty value names no executable" },
    });
  }
  if (named !== undefined) {
    return {
      path: named,
      notFoundFix: `${ENGINE_PATH_VARIABLE} names ${named}, which is no executable file: point it at Collie's executable, ${EXECUTABLE}, or leave it unset to use the one @goodfetch/collie installed.`,
    };
  }
  const platformPackage = `@goodfetch/collie-${process.platform}-${process.arch}`;
  const executable = process.platform === "win32" ? `${EXECUTABLE}.exe` : EXECUTABLE;
  const fix = `Install @goodfetch/collie with its optional dependencies, which hold its engine in ${platformPackage}, or set ${ENGINE_PATH_VARIABLE} to Collie's executable.`;
  let manifest: string;
  try {
    manifest = createRequire(import.meta.url).resolve(`${platformPackage}/package.json`);
  } catch (cause) {
    throw new EngineNotFoundError(`Collie's engine is not installed: no ${platformPackage}`, {
      fix,
      details: { package: platformPackage },
      cause,
    });
  }
  return { path: join(dirname(manifest), "bin", executable), notFoundFix: fix };
}
