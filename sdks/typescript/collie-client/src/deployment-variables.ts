// The environment map's strict reading at `connect()` (ADR 0083), as the
// engine reads its own: the `GOODFETCH_` prefix is reserved, so a variable
// off the closed list of deployment variables is `setting_unknown`, naming
// the closest one, and an empty or ill-formed value, or one the engine
// cannot parse, `setting_invalid`. Python's `_deployment_variables.py` is
// its twin.
import { isIPv4, isIPv6 } from "node:net";
import { CHANNELS, DEPLOYMENT_PREFIX, DEPLOYMENT_VARIABLES } from "./generated/connection.ts";
import { SettingInvalidError, SettingUnknownError } from "./generated/index.ts";

const deploymentVariables: ReadonlySet<string> = new Set(DEPLOYMENT_VARIABLES);
const channels: ReadonlySet<string> = new Set(CHANNELS);
const CHANNEL = "GOODFETCH_CHANNEL";
const DRAIN_GRACE = "GOODFETCH_DAEMON_DRAIN_GRACE";
const DAEMON_ADDRESS = "GOODFETCH_DAEMON_ADDRESS";
const OPS_ADDRESS = "GOODFETCH_OPS_ADDRESS";
/** The pod's Fargate profile label, which the downward API leaves empty off Fargate. */
const EKS_FARGATE_PROFILE = "GOODFETCH_EKS_FARGATE_PROFILE";
/** A whole number of milliseconds as Rust's `u64::from_str` reads it. */
const MILLISECONDS = /^\+?[0-9]+$/u;
/** The largest drain grace the engine reads, its `u64`'s. */
const LARGEST_MILLISECONDS = 2n ** 64n - 1n;
/**
 * An address and port in the shape Rust's `SocketAddr::from_str` reads: an IPv4 address, or an
 * IPv6 one in brackets with an optional decimal scope, then `:` and a decimal port; `isIP` then
 * checks the address itself.
 */
const SOCKET_ADDRESS =
  /^(?:\[(?<v6>[0-9A-Fa-f:.]+)(?:%(?<scope>[0-9]+))?\]|(?<v4>[0-9.]+)):(?<port>[0-9]+)$/u;
const LARGEST_PORT = 65_535n;
const LARGEST_SCOPE = 2n ** 32n - 1n;

/**
 * Checks every `GOODFETCH_` variable of `env` against the closed list of
 * deployment variables, the first by name failing.
 *
 * @throws {SettingUnknownError} for a variable not on the list.
 * @throws {SettingInvalidError} for an empty value but `GOODFETCH_EKS_FARGATE_PROFILE`'s, one that is no well-formed Unicode, a
 *   `GOODFETCH_CHANNEL` that names no channel, a `GOODFETCH_DAEMON_DRAIN_GRACE` that is no
 *   whole number of milliseconds or a `GOODFETCH_DAEMON_ADDRESS` or `GOODFETCH_OPS_ADDRESS`
 *   that is no IP address and port.
 */
export function checkDeploymentVariables(env: Readonly<Record<string, string>>): void {
  for (const name of Object.keys(env).toSorted()) {
    if (!name.startsWith(DEPLOYMENT_PREFIX)) {
      continue;
    }
    if (!deploymentVariables.has(name)) {
      throw unknownVariable(name);
    }
    const reason = invalidReason(name, env[name] ?? "");
    if (reason !== undefined) {
      throw new SettingInvalidError(`\`${name}\` is not valid in the environment: ${reason}`, {
        fix: `Set ${name} to ${takenValue(name)}, or unset it.`,
        details: { key: name, reason },
      });
    }
  }
}

/** Why `value` cannot be the deployment variable `name`'s, if it cannot. */
function invalidReason(name: string, value: string): string | undefined {
  if (value === "") {
    return name === EKS_FARGATE_PROFILE ? undefined : "it is empty; unset it instead";
  }
  if (!value.isWellFormed()) {
    return "it is not Unicode";
  }
  if (name === CHANNEL && !channels.has(value)) {
    return `\`${value}\` is no channel; one of ${CHANNELS.join(", ")}`;
  }
  if (
    name === DRAIN_GRACE &&
    !(MILLISECONDS.test(value) && BigInt(value) <= LARGEST_MILLISECONDS)
  ) {
    return `\`${value}\` is no whole number of milliseconds`;
  }
  if ((name === DAEMON_ADDRESS || name === OPS_ADDRESS) && !isSocketAddress(value)) {
    return `\`${value}\` is no \`host:port\``;
  }
  return undefined;
}

/** Whether Rust's `SocketAddr::from_str`, which the engine reads an address with, reads `value`. */
function isSocketAddress(value: string): boolean {
  const parts = SOCKET_ADDRESS.exec(value)?.groups;
  if (parts?.port === undefined || BigInt(parts.port) > LARGEST_PORT) {
    return false;
  }
  if (parts.v4 !== undefined) {
    return isIPv4(parts.v4);
  }
  return (
    parts.v6 !== undefined &&
    isIPv6(parts.v6) &&
    (parts.scope === undefined || BigInt(parts.scope) <= LARGEST_SCOPE)
  );
}

/** What the deployment variable `name` takes, as its `fix` says it. */
function takenValue(name: string): string {
  switch (name) {
    case CHANNEL:
      return `one of ${CHANNELS.join(", ")}`;
    case DRAIN_GRACE:
      return "a whole number of milliseconds";
    case DAEMON_ADDRESS:
      return "an IP address and port such as 127.0.0.1:7900";
    case OPS_ADDRESS:
      return "an IP address and port such as 0.0.0.0:9464";
    default:
      return "a non-empty Unicode value";
  }
}

/** The error for `name`, a `GOODFETCH_` variable off the list. */
function unknownVariable(name: string): SettingUnknownError {
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
