// Whether a Client's executable would be its major's pick once it ran
// (ADR 0098), read from the list of installed executables in a real
// Goodfetch home: the decision `installed::pick` makes, which a refusal's
// `fix` names Foxhound by.
import { mkdir, mkdtemp, rm, utimes, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import type { Product } from "./generated/types.ts";
import { wouldBeThePick } from "./installed-executables.ts";

const NOW = new Date("2026-11-21T12:00:00Z");
/** Seen 29 days, 23 hours, 59 minutes and 59 seconds before `NOW`: it still counts. */
const SEEN_LATELY = "2026-10-22T12:00:01Z";
/** Seen 30 days before `NOW`, the default `browser_cache.max_unused_age`: it no longer counts. */
const SEEN_30_DAYS_AGO = "2026-10-22T12:00:00Z";
const MODIFIED_AT = "2026-01-02T08:00:00Z";
const EXECUTABLE_BYTES = "foxhound";
const COPY = {
  kind: "executable_copy",
  name: "goodfetch-foxhound",
  build: "0.4.0-0123456789abcdef",
};

let home: string;
let executable: string;

beforeEach(async () => {
  home = await mkdtemp(join(tmpdir(), "installed-executables-"));
  executable = join(home, "bin", "goodfetch-foxhound");
  await mkdir(join(home, "bin"));
  await writeFile(executable, EXECUTABLE_BYTES);
  const modified = new Date(MODIFIED_AT);
  await utimes(executable, modified, modified);
});

afterEach(async () => {
  await rm(home, { recursive: true, force: true });
});

/** A Foxhound executable's entry at 0.4.0, at `executable` as it is, seen lately; `fields` replace its own. */
function entry(fields: Readonly<Record<string, unknown>> = {}): Record<string, unknown> {
  return {
    kind: "product_executable",
    product: "foxhound",
    product_version: "0.4.0",
    path: executable,
    size: EXECUTABLE_BYTES.length,
    modified_at: MODIFIED_AT,
    pins: [],
    last_seen: SEEN_LATELY,
    ...fields,
  };
}

/** An entry whose executable is gone from its path, at `last_seen`, with `copy`. */
function gone(last_seen: string, copy: unknown): Record<string, unknown> {
  return entry({ path: join(home, "bin", "removed"), last_seen, copy });
}

async function list(entries: readonly unknown[], format_version: unknown = "0.1"): Promise<void> {
  await writeFile(
    join(home, "installed-executables.json"),
    JSON.stringify({ format_version, executables: entries }),
  );
}

/** Puts the versioned copy `COPY` in the Browser cache at `root`. */
async function copyIn(root: string): Promise<void> {
  const folder = join(root, COPY.kind, COPY.name, COPY.build);
  await mkdir(join(folder, "files"), { recursive: true });
  await writeFile(
    join(folder, "entry.json"),
    JSON.stringify({ format_version: "0.1", artifact: COPY }),
  );
}

const collie = (version: string) => ({ product: "collie" as Product, version });

describe("wouldBeThePick", () => {
  test(
    "is the pick with no list of installed executables",
    { tags: ["story:11-293"] },
    async () => {
      expect(await wouldBeThePick(collie("0.4.0"), home, {}, NOW)).toBe(true);
    },
  );

  test(
    "is not the pick while a Foxhound of the line is at its path",
    { tags: ["story:11-293"] },
    async () => {
      await list([entry()]);

      expect(await wouldBeThePick(collie("0.4.0"), home, {}, NOW)).toBe(false);
    },
  );

  test(
    "is not the pick while a Foxhound gone from its path, seen lately, has its copy in the Browser cache",
    { tags: ["story:11-293"] },
    async () => {
      await list([gone(SEEN_LATELY, COPY)]);
      await copyIn(join(home, "browsers"));

      expect(await wouldBeThePick(collie("0.4.0"), home, {}, NOW)).toBe(false);
    },
  );

  test(
    "reads the copy in the Browser cache GOODFETCH_BROWSERS_PATH names",
    { tags: ["story:11-293"] },
    async () => {
      const cache = join(home, "elsewhere");
      await list([gone(SEEN_LATELY, COPY)]);
      await copyIn(cache);

      expect(
        await wouldBeThePick(collie("0.4.0"), home, { GOODFETCH_BROWSERS_PATH: cache }, NOW),
      ).toBe(false);
    },
  );

  test(
    "is not the pick while a Foxhound seen in the future has its copy",
    { tags: ["story:11-293"] },
    async () => {
      await list([gone("2026-12-01T00:00:00Z", COPY)]);
      await copyIn(join(home, "browsers"));

      expect(await wouldBeThePick(collie("0.4.0"), home, {}, NOW)).toBe(false);
    },
  );

  test.for([
    ["its copy is not in the Browser cache", () => gone(SEEN_LATELY, COPY)],
    ["it has no copy", () => gone(SEEN_LATELY, null)],
    ["it was last seen 30 days ago", () => gone(SEEN_30_DAYS_AGO, COPY)],
    [
      "its file at its path was rewritten and it was last seen 30 days ago",
      () => entry({ size: 9, last_seen: SEEN_30_DAYS_AGO }),
    ],
    [
      "its file's time differs by a second and it was last seen 30 days ago",
      () => entry({ modified_at: "2026-01-02T07:59:59Z", last_seen: SEEN_30_DAYS_AGO }),
    ],
    ["it is of another line", () => entry({ product_version: "0.5.0" })],
    ["it is a linked program", () => entry({ kind: "linked_program" })],
    ["its entry cannot be read", () => entry({ product_version: "0.4" })],
  ] as const)(
    "is the pick while a Foxhound is installed but %s",
    { tags: ["story:11-293"] },
    async ([, foxhound]) => {
      await list([foxhound()]);
      await copyIn(join(home, "unused"));

      expect(await wouldBeThePick(collie("0.4.0"), home, {}, NOW)).toBe(true);
    },
  );

  test.for([
    ["a list of another major", () => list([entry()], "1.0")],
    ["a list with no format version", () => list([entry()], null)],
    ["a list that is not JSON", () => writeFile(join(home, "installed-executables.json"), "{")],
  ] as const)("is the pick with %s", { tags: ["story:11-293"] }, async ([, write]) => {
    await write();

    expect(await wouldBeThePick(collie("0.4.0"), home, {}, NOW)).toBe(true);
  });

  test.for([
    ["a newer patch", "1.0.0-rc.2", "1.0.1", false],
    ["patch 10 above patch 9", "1.0.9", "1.0.10", false],
    ["a newer minor of the major", "1.2.0", "1.10.0", false],
    ["the release above its pre-release", "1.0.0-rc.2", "1.0.0", false],
    ["pre-release numbers compared as numbers", "1.0.0-rc.2", "1.0.0-rc.10", false],
    ["a word above a number", "1.0.0-1", "1.0.0-alpha", false],
    ["a longer pre-release above one it begins", "1.0.0-rc", "1.0.0-rc.1", false],
    ["an older pre-release", "1.0.0-rc.2", "1.0.0-rc.1", true],
    ["an older release", "1.4.0", "1.3.9", true],
    ["its own version", "1.4.0", "1.4.0", true],
  ] as const)(
    "with a Collie at %s installed, is the pick: %s",
    { tags: ["story:11-293"] },
    async ([, own, installed, pick]) => {
      await list([entry({ product: "collie", product_version: installed })]);

      expect(await wouldBeThePick(collie(own), home, {}, NOW)).toBe(pick);
    },
  );

  test.for([
    ["a newer Foxhound", "0.4.1", false],
    ["its own version", "0.4.0", true],
  ] as const)(
    "a Foxhound Client with %s installed is the pick: %s",
    { tags: ["story:11-293"] },
    async ([, installed, pick]) => {
      await list([entry({ product_version: installed })]);

      expect(await wouldBeThePick({ product: "foxhound", version: "0.4.0" }, home, {}, NOW)).toBe(
        pick,
      );
    },
  );
});
