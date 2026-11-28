// Unit tests for adapter peer ranges and the `adapters-latest` Live check
// (ADRs 0064, 0070, 0072; spec #608, issue 1382).
import { existsSync, readFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";
import {
  classifyRegistryVersions,
  discoverAdapterPeers,
  INCONCLUSIVE_EXIT,
  parseDeclaredRange,
  planPeerVersions,
  runAdaptersLatest,
} from "../../../../ci/live/adapters-latest.ts";

const REPO_ROOT = resolve(fileURLToPath(new URL("../../../..", import.meta.url)));

function readJson(path: string): Record<string, unknown> {
  const parsed: unknown = JSON.parse(readFileSync(path, "utf8"));
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
    throw new TypeError(`expected JSON object in ${path}`);
  }
  const result: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(parsed)) {
    result[key] = value;
  }
  return result;
}

const fakeRegistryFetch: typeof fetch = async (input) => {
  const url = typeof input === "string" ? input : input instanceof URL ? input.href : input.url;
  if (url.includes("registry.npmjs.org/ai")) {
    return new Response(
      JSON.stringify({
        versions: { "7.0.0": {}, "7.0.99": {}, "8.0.0-beta.1": {}, "8.0.0": {} },
      }),
      { status: 200 },
    );
  }
  if (url.includes("registry.npmjs.org/")) {
    return new Response(
      JSON.stringify({ versions: { "1.0.0": {}, "1.5.0": {}, "2.0.0-rc.1": {} } }),
      { status: 200 },
    );
  }
  return new Response(
    JSON.stringify({ releases: { "1.0.0": [], "1.3.0": [], "2.0.0a1": [], "2.0.0": [] } }),
    { status: 200 },
  );
};

const unavailableRegistryFetch: typeof fetch = async () =>
  new Response("unavailable", { status: 503 });

describe("Collie adapter peer ranges and adapters-latest Live check", () => {
  test(
    "every Collie adapter states its declared peer range in its README and the release-notes template",
    { tags: ["story:11-244"] },
    () => {
      const peers = discoverAdapterPeers(REPO_ROOT).filter((entry) => entry.product === "collie");
      expect(peers.length).toBeGreaterThanOrEqual(4);

      const releaseNotesTemplate = readFileSync(
        join(REPO_ROOT, "docs", "release-notes-template.md"),
        "utf8",
      );

      for (const entry of peers) {
        const readme = readFileSync(join(entry.adapterDir, "README.md"), "utf8");
        expect(readme, `${entry.packageName} README must name ${entry.peerName}`).toContain(
          entry.peerName,
        );
        expect(readme, `${entry.packageName} README must state ${entry.range}`).toContain(
          entry.range,
        );
        expect(
          releaseNotesTemplate,
          `release-notes template must name ${entry.packageName}`,
        ).toContain(entry.packageName);
        expect(
          releaseNotesTemplate,
          `release-notes template must state ${entry.range} for ${entry.packageName}`,
        ).toContain(entry.range);
      }

      const npmPeers = peers.filter((entry) => entry.ecosystem === "npm");
      expect(npmPeers.length).toBeGreaterThanOrEqual(3);
      for (const entry of npmPeers) {
        const lowestPkgPath = join(entry.adapterDir, "tests", "lowest-version", "package.json");
        expect(
          existsSync(lowestPkgPath),
          `${entry.packageName} must have tests/lowest-version/package.json`,
        ).toBe(true);
        const lowestPkg = readJson(lowestPkgPath);
        const deps = lowestPkg["dependencies"];
        const bounds = parseDeclaredRange(entry.range, "npm");
        const pinned =
          typeof deps === "object" && deps !== null && !Array.isArray(deps)
            ? Reflect.get(deps, entry.peerName)
            : undefined;
        expect(pinned).toBe(bounds.lowestVersion);
      }
    },
  );

  test(
    "plans lowest, highest, and one version per major inside multi-major ranges",
    { tags: ["story:11-244"] },
    () => {
      const singleNpm = planPeerVersions("^7.0.0", "npm", [
        "6.9.0",
        "7.0.0",
        "7.0.50",
        "7.0.99",
        "8.0.0-alpha.1",
      ]);
      expect(singleNpm).toEqual({
        lowest: "7.0.0",
        highest: "7.0.99",
        interiorMajors: [],
        selected: ["7.0.0", "7.0.99"],
      });

      const multiNpm = planPeerVersions("^5.0.0 || ^6.0.0 || ^7.0.0", "npm", [
        "5.0.0",
        "5.4.1",
        "6.0.0",
        "6.8.2",
        "7.0.0",
        "7.1.3",
        "8.0.0",
      ]);
      expect(multiNpm).toEqual({
        lowest: "5.0.0",
        highest: "7.1.3",
        interiorMajors: ["6.8.2"],
        selected: ["5.0.0", "6.8.2", "7.1.3"],
      });

      const multiPypi = planPeerVersions(">=1.0.0,<4", "pypi", [
        "1.0.0",
        "1.2.0",
        "2.0.0",
        "2.5.1",
        "3.0.0",
        "3.4.0",
        "4.0.0a1",
      ]);
      expect(multiPypi).toEqual({
        lowest: "1.0.0",
        highest: "3.4.0",
        interiorMajors: ["2.5.1"],
        selected: ["1.0.0", "2.5.1", "3.4.0"],
      });
    },
  );

  test(
    "adapters-latest Live check is registered and reports new majors and prereleases without blocking",
    { tags: ["story:11-244"] },
    async () => {
      const liveChecksToml = readFileSync(join(REPO_ROOT, "ci", "live-checks.toml"), "utf8");
      expect(liveChecksToml).toContain('id = "adapters-latest"');
      expect(liveChecksToml).toContain('job = "nightly.yml#releases"');
      expect(liveChecksToml).toContain("area = 11");

      const classified = classifyRegistryVersions("^7.0.0", "npm", [
        "7.0.0",
        "7.0.99",
        "8.0.0-rc.1",
        "8.0.0",
      ]);
      expect(classified.latestInRange).toBe("7.0.99");
      expect(classified.latestNewMajor).toBe("8.0.0");
      expect(classified.latestPrerelease).toBe("8.0.0-rc.1");

      const outcome = await runAdaptersLatest({
        repoRoot: REPO_ROOT,
        fetchImpl: fakeRegistryFetch,
      });
      expect(outcome.exitCode).toBe(0);
      expect(outcome.report).toContain("new major: 8.0.0");
      expect(outcome.report).toContain("prerelease: 8.0.0-beta.1");
      expect(outcome.report).toContain("new major: 2.0.0");
      expect(outcome.report).toContain("prerelease: 2.0.0a1");

      const failedOutcome = await runAdaptersLatest({
        repoRoot: REPO_ROOT,
        fetchImpl: unavailableRegistryFetch,
      });
      expect(failedOutcome.exitCode).toBe(INCONCLUSIVE_EXIT);
    },
  );
});
