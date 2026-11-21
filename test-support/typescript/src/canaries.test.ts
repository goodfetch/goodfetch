import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, test } from "vitest";
import { type Canary, type CanaryHit, searchCanaries, type SecretKind } from "./canaries.ts";
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

const folders: string[] = [];

afterEach(() => {
  for (const folder of folders.splice(0)) rmSync(folder, { recursive: true, force: true });
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
