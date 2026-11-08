import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { parse } from "smol-toml";
import { afterEach, describe, expect, test } from "vitest";
import {
  type Canary,
  type CanaryHit,
  SECRET_KINDS,
  searchCanaries,
  type SecretKind,
} from "./canaries.ts";
import { TestSupportError } from "./errors.ts";

/** The shared fixture set the Rust and Python searches read too. */
interface Fixture {
  canaries: { id: string; kind: SecretKind; value: string; planted_in?: string[] }[];
  outputs: { name: string; text: string }[];
  hits: CanaryHit[];
}

const fixture: Fixture = JSON.parse(
  readFileSync(new URL("../../fixtures/canaries/search.json", import.meta.url), "utf8"),
);

const canaries: Canary[] = fixture.canaries.map(({ id, kind, value, planted_in }) => ({
  id,
  kind,
  value,
  plantedIn: planted_in,
}));

/** The rule IDs of the `gitleaks` rule `xtask codegen` writes from Rust's kinds, one per kind. */
function canaryRuleIds(): string[] {
  const { rules } = parse(
    readFileSync(new URL("../../fixtures/canaries/gitleaks.toml", import.meta.url), "utf8"),
  );
  if (!Array.isArray(rules)) throw new Error("the canary rule holds no rules");
  return rules.map((rule) => {
    const id = typeof rule === "object" && "id" in rule ? rule.id : undefined;
    if (typeof id !== "string") throw new Error("a canary rule has no ID");
    return id;
  });
}

const folders: string[] = [];

afterEach(() => {
  for (const folder of folders.splice(0)) rmSync(folder, { recursive: true, force: true });
});

test("SECRET_KINDS names every kind Rust test support plants, in its order", () => {
  const planted = canaryRuleIds().map((id) =>
    id.replace(/^goodfetch-secret-canary-/, "").replaceAll("-", "_"),
  );

  expect(SECRET_KINDS).toEqual(planted);
});

describe("searchCanaries", () => {
  test.for(fixture.outputs.map((output) => [output.name, output] as const))(
    "finds each hit of the shared fixture in %s",
    ([name, output]) => {
      const hits = searchCanaries(canaries, [output]);

      expect(hits).toEqual(fixture.hits.filter((hit) => hit.output === name));
    },
  );

  test("names the file and offset of a canary in a folder", () => {
    const folder = mkdtempSync(join(tmpdir(), "canaries "));
    folders.push(folder);
    mkdirSync(join(folder, "logs"));
    writeFileSync(join(folder, "clean.txt"), "nothing here");
    writeFileSync(join(folder, "logs", "sdk.log"), "Error: token=fixture-session-cookie-2\n");

    const hits = searchCanaries(canaries, [{ path: folder }]);

    expect(hits).toEqual([
      {
        canary: "session",
        kind: "session_cookie",
        encoding: "raw",
        output: join(folder, "logs", "sdk.log"),
        offset: 13,
        placement: "leaked",
      },
    ]);
  });

  test("fails with canary_output_unreadable on a missing path", () => {
    const folder = mkdtempSync(join(tmpdir(), "canaries "));
    folders.push(folder);

    const search = (): CanaryHit[] => searchCanaries(canaries, [{ path: join(folder, "missing") }]);

    expect(search).toThrow(
      expect.objectContaining<Partial<TestSupportError>>({ code: "canary_output_unreadable" }),
    );
  });
});
