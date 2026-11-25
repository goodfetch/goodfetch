import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterAll, beforeAll, describe, expect, test } from "vitest";
import { searchOptions } from "./search.ts";

interface Schema {
  validate(options: unknown): { value: unknown; error?: Error };
}

interface SearchLocal {
  default(context: unknown, options: unknown): { postBuild(buildData: unknown): Promise<void> };
  validateOptions(args: {
    validate: (schema: Schema, options: unknown) => unknown;
    options: unknown;
  }): unknown;
}

// The package types its options only, so its two exports are checked here.
function isSearchLocal(value: unknown): value is SearchLocal {
  return (
    typeof value === "object" &&
    value !== null &&
    "default" in value &&
    typeof value.default === "function" &&
    "validateOptions" in value &&
    typeof value.validateOptions === "function"
  );
}

const loaded: unknown = createRequire(import.meta.url)("@easyops-cn/docusaurus-search-local");
if (!isSearchLocal(loaded)) {
  throw new Error("@easyops-cn/docusaurus-search-local lacks its plugin or its option check");
}
const searchLocal = loaded;

// Two kept versions, each with a root page and both sections; the URL is
// each page's route, as the site serves it with `trailingSlash: false`.
const versions = [
  {
    path: "/docs",
    isLast: true,
    pages: [
      "/docs/",
      "/docs/collie",
      "/docs/collie/install",
      "/docs/foxhound",
      "/docs/foxhound/runs",
    ],
  },
  {
    path: "/docs/next",
    isLast: false,
    pages: ["/docs/next/", "/docs/next/collie", "/docs/next/foxhound"],
  },
];

let site: string;

/** The pages an index at `file` of the built site holds, by URL. */
function indexed(file: string): string[] {
  // An index is a list of Lunr indexes, one per kind of entry, each with
  // its `documents`, whose `u` is a page's URL.
  const urls = new Set<string>();
  const types: unknown = JSON.parse(readFileSync(join(site, "build", file), "utf8"));
  for (const type of Array.isArray(types) ? (types as unknown[]) : []) {
    const documents: unknown =
      typeof type === "object" && type !== null && "documents" in type ? type.documents : [];
    for (const document of Array.isArray(documents) ? (documents as unknown[]) : []) {
      if (typeof document === "object" && document !== null && "u" in document) {
        urls.add(String(document.u));
      }
    }
  }
  return [...urls].toSorted();
}

beforeAll(async () => {
  site = mkdtempSync(join(tmpdir(), "search-"));
  const outDir = join(site, "build");
  for (const url of versions.flatMap((version) => version.pages)) {
    const route = url.slice("/docs/".length).replace(/\/$/, "");
    const file = join(outDir, route === "" ? "index.html" : `${route}.html`);
    mkdirSync(dirname(file), { recursive: true });
    writeFileSync(
      file,
      `<html><body><article><h1>${url}</h1><p>Crawl the page.</p></article></body></html>`,
    );
  }
  const options = searchLocal.validateOptions({
    validate: (schema, given) => {
      const { value, error } = schema.validate(given);
      if (error !== undefined) {
        throw error;
      }
      return value;
    },
    options: searchOptions,
  });
  const plugin = searchLocal.default(
    {
      siteDir: site,
      generatedFilesDir: join(site, ".docusaurus"),
      baseUrl: "/docs/",
      i18n: { currentLocale: "en" },
      siteConfig: { themeConfig: {}, trailingSlash: false },
    },
    options,
  );
  await plugin.postBuild({
    outDir,
    baseUrl: "/docs/",
    routesPaths: versions.flatMap((version) => version.pages),
    siteConfig: { trailingSlash: false },
    plugins: [
      {
        name: "docusaurus-plugin-content-docs",
        content: {
          loadedVersions: versions.map((version) => ({
            path: version.path,
            isLast: version.isLast,
            docs: version.pages.map((permalink) => ({ permalink })),
          })),
        },
      },
    ],
  });
});

afterAll(() => {
  rmSync(site, { recursive: true, force: true });
});

describe("the search index", { tags: ["story:1-119"] }, () => {
  test("of a section holds its pages and the root pages, and none of the other section", () => {
    expect(indexed("search-index-collie.json")).toEqual([
      "/docs/",
      "/docs/collie",
      "/docs/collie/install",
    ]);
    expect(indexed("search-index-foxhound.json")).toEqual([
      "/docs/",
      "/docs/foxhound",
      "/docs/foxhound/runs",
    ]);
  });

  test("of the root holds only the root pages", () => {
    expect(indexed("search-index.json")).toEqual(["/docs/"]);
  });

  test("of a section of `next` sits in its version and holds that version's pages", () => {
    expect(indexed("next/search-index-collie.json")).toEqual(["/docs/next/", "/docs/next/collie"]);
  });
});
