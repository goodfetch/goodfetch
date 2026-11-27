// Settings completeness test (spec #611's #1500; ADR 0083): every Foxhound
// setting key of the schema (`foxhound.json`) is present in Foxhound's
// `Settings` types (`FoxhoundSettings`, `CrawlSettings`, `LabelSettings`) with
// its exact allowed scopes, and none that the schema omits (such as
// configuration-only keys). Collie's `politeness.proxy.*`, `politeness.sharing`
// and `site_preferences.respect_use_signals` stay defined in `@goodfetch/collie-client`.
import { readFileSync } from "node:fs";
import { describe, expect, test } from "vitest";
import { settingsOnWire } from "./generated/settings.ts";

interface CatalogSetting {
  readonly name: string;
  readonly product: "collie" | "foxhound";
  readonly scopes: readonly ("connection" | "run" | "label" | "session" | "page")[];
}

// oxlint-disable-next-line typescript/no-unsafe-type-assertion -- committed catalog JSON lists every setting with its product and allowed scopes.
const catalog = JSON.parse(
  readFileSync(
    new URL("../../../../crates/goodfetch-protocol/generated/foxhound.json", import.meta.url),
    "utf8",
  ),
) as { readonly settings: readonly CatalogSetting[] };

/**
 * Extracts every wire setting key and its camelCase property path from the
 * generated interface `interfaceName` in `fileUrl`.
 */
function keysOfSettingsInterface(fileUrl: URL, interfaceName: string): Map<string, string[]> {
  const lines = readFileSync(fileUrl, "utf8").split("\n");
  const startPattern = new RegExp(`^export interface ${interfaceName} \\{$`);
  const start = lines.findIndex((line) => startPattern.test(line));
  if (start === -1) {
    throw new Error(`interface ${interfaceName} not found in ${fileUrl.pathname}`);
  }

  const found = new Map<string, string[]>();
  const path: string[] = [];
  let pendingWireKey: string | null = null;
  let insideBlockDepth = 0;

  for (let i = start + 1; i < lines.length; i++) {
    const line = lines[i] ?? "";
    if (line === "}") {
      break;
    }
    const wireMatch = /^\s*\*\s*`([a-z0-9_]+(?:\.[a-z0-9_]+)*)`\.\s*$/u.exec(line);
    if (wireMatch?.[1] !== undefined) {
      pendingWireKey = wireMatch[1];
      continue;
    }
    if (insideBlockDepth > 0) {
      // Skip nested inline value-type object braces inside a single setting's type.
      for (const ch of line) {
        if (ch === "{") {
          insideBlockDepth++;
        } else if (ch === "}") {
          insideBlockDepth--;
        }
      }
      continue;
    }
    const openGroup = /^\s+readonly ([a-zA-Z0-9_]+)\?: \{$/u.exec(line);
    if (openGroup?.[1] !== undefined) {
      if (pendingWireKey !== null) {
        found.set(pendingWireKey, [...path, openGroup[1]]);
        pendingWireKey = null;
        insideBlockDepth = 1;
      } else {
        path.push(openGroup[1]);
      }
      continue;
    }
    if (/^\s+\};$/u.test(line)) {
      path.pop();
      continue;
    }
    const leafProp = /^\s+readonly ([a-zA-Z0-9_]+)\?:/u.exec(line);
    if (leafProp?.[1] !== undefined && pendingWireKey !== null) {
      found.set(pendingWireKey, [...path, leafProp[1]]);
      pendingWireKey = null;
    }
  }
  return found;
}

const FOXHOUND_SETTINGS_FILE = new URL("./generated/settings.ts", import.meta.url);
const COLLIE_SETTINGS_FILE = new URL(
  "../../collie-client/src/generated/settings.ts",
  import.meta.url,
);

describe("Foxhound Settings schema completeness", () => {
  test.for([
    { scope: "connection" as const, interfaceName: "FoxhoundSettings" },
    { scope: "run" as const, interfaceName: "CrawlSettings" },
    { scope: "label" as const, interfaceName: "LabelSettings" },
  ])(
    "$interfaceName has every schema setting allowed at scope $scope and none omitted",
    ({ scope, interfaceName }) => {
      const expected = catalog.settings
        .filter((setting) => setting.scopes.includes(scope))
        .map((setting) => setting.name)
        .toSorted();

      const actual = keysOfSettingsInterface(FOXHOUND_SETTINGS_FILE, interfaceName);
      expect([...actual.keys()].toSorted()).toEqual(expected);

      // Configuration-only keys (empty scopes) must never appear in code settings.
      const configOnly = catalog.settings
        .filter((setting) => setting.scopes.length === 0)
        .map((setting) => setting.name);
      for (const key of configOnly) {
        expect(actual.has(key)).toBe(false);
      }
    },
  );

  test("settingsOnWire maps every camelCase key in FoxhoundSettings to its wire snake_case key", () => {
    const keys = keysOfSettingsInterface(FOXHOUND_SETTINGS_FILE, "FoxhoundSettings");
    for (const [wireKey, camelPath] of keys) {
      const nested: Record<string, unknown> = {};
      let cursor = nested;
      for (let i = 0; i < camelPath.length - 1; i++) {
        const segment = camelPath[i];
        if (segment === undefined) {
          continue;
        }
        const child: Record<string, unknown> = {};
        cursor[segment] = child;
        cursor = child;
      }
      const leaf = camelPath.at(-1);
      if (leaf !== undefined) {
        cursor[leaf] = "sentinel";
      }

      const wire = settingsOnWire(nested);
      let wireCursor: unknown = wire;
      for (const seg of wireKey.split(".")) {
        expect(wireCursor).toBeTypeOf("object");
        expect(wireCursor).not.toBeNull();
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- traversing the plain JSON object returned by settingsOnWire.
        wireCursor = (wireCursor as Record<string, unknown>)[seg];
      }
      expect(wireCursor).toBe("sentinel");
    }
  });

  test("Collie's politeness and site_preferences keys remain in @goodfetch/collie-client", () => {
    const collieKeys = keysOfSettingsInterface(COLLIE_SETTINGS_FILE, "Settings");
    const foxhoundKeys = keysOfSettingsInterface(FOXHOUND_SETTINGS_FILE, "FoxhoundSettings");

    for (const key of [
      "politeness.proxy.concurrency",
      "politeness.proxy.rate",
      "politeness.sharing",
      "site_preferences.respect_use_signals",
    ]) {
      expect(collieKeys.has(key)).toBe(true);
      expect(foxhoundKeys.has(key)).toBe(true);
    }
    expect(collieKeys.has("politeness.domain.concurrency")).toBe(false);
    expect(foxhoundKeys.has("politeness.domain.concurrency")).toBe(true);
    expect(foxhoundKeys.has("politeness.degraded_max")).toBe(false);
  });
});
