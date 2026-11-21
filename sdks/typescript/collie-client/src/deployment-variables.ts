// The environment map's strict reading at `connect()` (ADR 0083), as the
// engine reads its own: the `GOODFETCH_` prefix is reserved, so a variable
// off the closed list of deployment variables is `setting_unknown`, naming
// the closest one, and an empty or ill-formed value `setting_invalid`.
import { DEPLOYMENT_PREFIX, DEPLOYMENT_VARIABLES } from "./generated/connection.ts";
import { SettingInvalidError, SettingUnknownError } from "./generated/index.ts";

const deploymentVariables: ReadonlySet<string> = new Set(DEPLOYMENT_VARIABLES);

/**
 * Checks every `GOODFETCH_` variable of `env` against the closed list of
 * deployment variables, the first by name failing.
 *
 * @throws {SettingUnknownError} for a variable not on the list.
 * @throws {SettingInvalidError} for an empty value or one that is no well-formed Unicode.
 */
export function checkDeploymentVariables(env: Readonly<Record<string, string>>): void {
  for (const name of Object.keys(env).toSorted()) {
    if (!name.startsWith(DEPLOYMENT_PREFIX)) {
      continue;
    }
    if (!deploymentVariables.has(name)) {
      throw unknown(name);
    }
    const value = env[name] ?? "";
    const reason =
      value === ""
        ? "it is empty; unset it instead"
        : value.isWellFormed()
          ? undefined
          : "it is not Unicode";
    if (reason !== undefined) {
      throw new SettingInvalidError(`\`${name}\` is not valid in the environment: ${reason}`, {
        fix: `Give ${name} a value, or unset it.`,
        details: { key: name, reason },
      });
    }
  }
}

function unknown(name: string): SettingUnknownError {
  const closest = nearest(name);
  if (closest === undefined) {
    return new SettingUnknownError(`\`${name}\` in the environment is no deployment variable.`, {
      fix: `Unset ${name}: the ${DEPLOYMENT_PREFIX} prefix is reserved for the deployment variables, ${DEPLOYMENT_VARIABLES.join(", ")}.`,
      details: { key: name },
    });
  }
  return new SettingUnknownError(
    `\`${name}\` in the environment is no deployment variable; did you mean \`${closest}\`?`,
    { fix: `Rename ${name} to ${closest}, or unset it.`, details: { key: name, closest } },
  );
}

/** The deployment variable closest to `name`, when one is within a third of its length in edits (at least two), as the engine finds it. */
function nearest(name: string): string | undefined {
  const allowed = Math.max(Math.floor(Array.from(name).length / 3), 2);
  let best: { readonly edits: number; readonly variable: string } | undefined;
  for (const variable of DEPLOYMENT_VARIABLES) {
    const edits = distance(name, variable);
    if (
      edits <= allowed &&
      (best === undefined ||
        edits < best.edits ||
        (edits === best.edits && variable < best.variable))
    ) {
      best = { edits, variable };
    }
  }
  return best?.variable;
}

/** Levenshtein's distance between `left` and `right`, by code points. */
function distance(left: string, right: string): number {
  const rightPoints = Array.from(right);
  let previous = Array.from({ length: rightPoints.length + 1 }, (_, column) => column);
  for (const [row, leftPoint] of Array.from(left).entries()) {
    const current = [row + 1];
    for (const [column, rightPoint] of rightPoints.entries()) {
      current.push(
        Math.min(
          (previous[column] ?? 0) + (leftPoint === rightPoint ? 0 : 1),
          (current[column] ?? 0) + 1,
          (previous[column + 1] ?? 0) + 1,
        ),
      );
    }
    previous = current;
  }
  return previous[rightPoints.length] ?? 0;
}
