// The docs versions the site builds (ADR 0030, story 1-39): `next` from
// `main`, the stable version at the site's root, and the last minor of each
// previous major that `versions.json` still holds, the release's version cut
// having dropped the unsupported ones. During `0.y` previews each minor
// counts as a major, as SemVer reads them. `next` and the previous majors
// carry a banner; stable carries none.
import { existsSync, readFileSync } from "node:fs";

type Banner = "none" | "unreleased" | "unmaintained";

export interface VersionOptions {
  label: string;
  path?: string;
  banner: Banner;
}

export interface VersionsConfig {
  lastVersion: string;
  onlyIncludeVersions: string[];
  versions: Record<string, VersionOptions>;
}

/** The versions `docusaurus docs:version` cut, newest first; none before the first release. */
export function readVersions(file: string): string[] {
  if (!existsSync(file)) {
    return [];
  }
  const versions: unknown = JSON.parse(readFileSync(file, "utf8"));
  if (!Array.isArray(versions) || !versions.every((version) => typeof version === "string")) {
    throw new Error(`${file} must list the cut versions as strings`);
  }
  return versions;
}

/** The major a version belongs to: its major, or `0.<minor>` before 1.0. */
function line(version: string): string {
  const [major = "", minor = ""] = version.split(".");
  return major === "0" ? `0.${minor}` : major;
}

/** The docs plugin's version options for the cut `versions`, newest first. */
export function versionsConfig(versions: readonly string[]): VersionsConfig {
  const [stable] = versions;
  if (stable === undefined) {
    return {
      lastVersion: "current",
      onlyIncludeVersions: ["current"],
      versions: { current: { label: "next", banner: "unreleased" } },
    };
  }
  const seen = new Set([line(stable)]);
  const previous = versions.filter((version) => {
    const major = line(version);
    if (seen.has(major)) {
      return false;
    }
    seen.add(major);
    return true;
  });
  return {
    lastVersion: stable,
    onlyIncludeVersions: ["current", stable, ...previous],
    versions: {
      current: { label: "next", path: "next", banner: "unreleased" },
      [stable]: { label: stable, banner: "none" },
      ...Object.fromEntries(
        previous.map((version) => [
          version,
          { label: version, path: version, banner: "unmaintained" as const },
        ]),
      ),
    },
  };
}
