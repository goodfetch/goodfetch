// The settings a Goodfetch adapter sets for its row (ADR 0025): a default
// row sets none, so every control stays on; a matched row sets the ones its
// campaign names, which switch off the controls the tool it is matched
// against lacks, and sets Filter-list blocking and Consent prompt handling
// as that tool's published configuration declares its defaults (ADRs 0047
// and 0048). Each setting is named by its dotted wire key; the SDKs take
// them nested by camelCase segment.
import type { Row } from "../../../src/campaign.ts";

/** Settings by their dotted wire key, such as `consent.handling`. */
export type FlatSettings = Readonly<Record<string, unknown>>;

/** The clean-up settings a matched row sets from the tool's published defaults, never from its campaign. */
const decidedByDefaults = [
  "filter_lists.ads",
  "filter_lists.trackers",
  "filter_lists.annoyances",
  "consent.handling",
] as const;

/** The settings `row` sets: the marks a run record keeps (ADR 0023). */
export function rowSettings(row: Row): FlatSettings {
  if (row.kind === "default") {
    return {};
  }
  for (const key of decidedByDefaults) {
    if (key in row.settings) {
      throw new Error(
        `the matched row against ${row.against} names ${key}, which the tool's published defaults decide`,
      );
    }
  }
  const blocking = row.competitor_defaults.filter_list_blocking;
  return {
    ...row.settings,
    "filter_lists.ads": blocking,
    "filter_lists.trackers": blocking,
    "filter_lists.annoyances": blocking,
    "consent.handling": row.competitor_defaults.consent_prompt_handling ? "answer" : "off",
  };
}

/**
 * The settings that let a Run reach the Proving ground: its task's host
 * resolves to the address the Proving ground listens on, and the egress
 * guard lets that host through, as `xtask compare` sets them.
 */
export function reachSettings(startUrl: string, address: string): FlatSettings {
  const host = new URL(startUrl).hostname;
  const ip = new URL(`http://${address}`).hostname.replace(/^\[(.*)\]$/, "$1");
  return {
    "network.resolve": { [host]: [ip] },
    "egress.private_ranges": [host],
  };
}

/** `settings` nested by camelCase segment, as the SDKs take them; a value's own keys stay as they are. */
export function nested(settings: FlatSettings): Record<string, unknown> {
  const root: Record<string, unknown> = {};
  // Each group made so far, by its dotted path.
  const groups = new Map<string, Record<string, unknown>>([["", root]]);
  for (const [key, value] of Object.entries(settings)) {
    const segments = key.split(".").map(camelCase);
    const last = segments.pop();
    if (last === undefined) {
      continue;
    }
    let scope = root;
    let path = "";
    for (const segment of segments) {
      path = `${path}.${segment}`;
      let group = groups.get(path);
      if (group === undefined) {
        group = {};
        groups.set(path, group);
        scope[segment] = group;
      }
      scope = group;
    }
    scope[last] = value;
  }
  return root;
}

function camelCase(segment: string): string {
  return segment.replace(/_([a-z0-9])/g, (_match, letter: string) => letter.toUpperCase());
}
