import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, test } from "vitest";
import { type Version, checkOutput, markdownFiles } from "./llms.ts";

const site = { title: "Goodfetch", description: "Browser automation and crawling" };

function page(permalink: string, source: string, title: string, text: string) {
  return { permalink, source, title, description: `About ${title}`, text };
}

const stable: Version = {
  path: "/docs",
  pages: [
    page(
      "/docs/",
      "/src/index.md",
      "Choose a Product",
      "# Choose a Product\n\nRead [Collie](collie/index.md).\n",
    ),
    page(
      "/docs/collie",
      "/src/collie/index.md",
      "Home",
      "---\ntitle: Home\n---\n\n# Home\n\n- [Install](install.md#steps)\n- [Choose](../index.md)\n- [Site](https://example.com/a.md)\n\n```text\n[kept](install.md)\n```\n",
    ),
    page("/docs/collie/install", "/src/collie/install.md", "Install", "## Steps\n\nRun it.\n"),
    page("/docs/foxhound", "/src/foxhound/index.md", "Home", "# Home\n\nFoxhound crawls.\n"),
  ],
};

const next: Version = {
  path: "/docs/next",
  pages: [
    page("/docs/next/", "/next/index.md", "Choose a Product", "# Choose a Product\n"),
    page(
      "/docs/next/collie",
      "/next/collie/index.md",
      "Home",
      "# Home\n\nSee [Install](install.md).\n",
    ),
    page("/docs/next/collie/install", "/next/collie/install.md", "Install", "# Install\n"),
    page("/docs/next/foxhound", "/next/foxhound/index.md", "Home", "# Home\n"),
  ],
};

const files = markdownFiles([stable, next], site);

describe("markdownFiles", { tags: ["story:1-44"] }, () => {
  test("writes each page's twin at its URL with `.md`, a home's as `index.md`, without front matter", () => {
    expect(files.get("/docs/index.md")).toBe(
      "# Choose a Product\n\nRead [Collie](/docs/collie.md).\n",
    );
    expect(files.get("/docs/collie.md")?.startsWith("# Home\n")).toBe(true);
    expect(files.get("/docs/next/collie/install.md")).toBe("# Install\n");
  });

  test("gives a twin whose page has no heading its title", () => {
    expect(files.get("/docs/collie/install.md")).toBe("# Install\n\n## Steps\n\nRun it.\n");
  });

  test("points a twin's links to pages at their twins in the same version, outside code blocks", () => {
    expect(files.get("/docs/collie.md")).toBe(
      "# Home\n\n- [Install](/docs/collie/install.md#steps)\n- [Choose](/docs/index.md)\n- [Site](https://example.com/a.md)\n\n```text\n[kept](install.md)\n```\n",
    );
    expect(files.get("/docs/next/collie.md")).toBe(
      "# Home\n\nSee [Install](/docs/next/collie/install.md).\n",
    );
  });

  test("lists a section's pages in its own llms.txt and holds their text in its llms-full.txt", () => {
    expect(files.get("/docs/collie/llms.txt")).toBe(
      [
        "# Goodfetch Collie",
        "",
        "> About Home",
        "",
        "## Docs",
        "",
        "- [Home](/docs/collie.md): About Home",
        "- [Install](/docs/collie/install.md): About Install",
        "",
      ].join("\n"),
    );
    const full = files.get("/docs/collie/llms-full.txt") ?? "";
    expect(full).toContain(files.get("/docs/collie.md"));
    expect(full).toContain(files.get("/docs/collie/install.md"));
    expect(full).not.toContain("Foxhound crawls.");
  });

  test("lists both sections' files and the root pages in the root's llms.txt, and only root pages in its llms-full.txt", () => {
    expect(files.get("/docs/llms.txt")).toBe(
      [
        "# Goodfetch",
        "",
        "> Browser automation and crawling",
        "",
        "## Products",
        "",
        "- [Goodfetch Collie](/docs/collie/llms.txt): the section's pages; their text is in [llms-full.txt](/docs/collie/llms-full.txt)",
        "- [Goodfetch Foxhound](/docs/foxhound/llms.txt): the section's pages; their text is in [llms-full.txt](/docs/foxhound/llms-full.txt)",
        "",
        "## Docs",
        "",
        "- [Choose a Product](/docs/index.md): About Choose a Product",
        "",
      ].join("\n"),
    );
    expect(files.get("/docs/llms-full.txt")).toBe(files.get("/docs/index.md"));
  });

  test("writes each version's files under its own path", () => {
    expect(files.get("/docs/next/collie/llms.txt")).toContain(
      "- [Install](/docs/next/collie/install.md): About Install",
    );
    expect(files.get("/docs/next/llms.txt")).toContain("(/docs/next/foxhound/llms.txt)");
  });
});

describe("checkOutput", { tags: ["story:1-44"] }, () => {
  let out: string | undefined;

  afterEach(() => {
    if (out !== undefined) {
      rmSync(out, { recursive: true, force: true });
    }
  });

  function written(entries: Map<string, string>): string {
    out = mkdtempSync(join(tmpdir(), "llms-"));
    for (const [url, text] of entries) {
      const file = join(out, url.slice("/docs/".length));
      mkdirSync(dirname(file), { recursive: true });
      writeFileSync(file, text);
    }
    return out;
  }

  const routes = [
    "/docs/",
    "/docs/collie",
    "/docs/collie/install",
    "/docs/foxhound",
    "/docs/next/",
    "/docs/next/collie",
    "/docs/next/collie/install",
    "/docs/next/foxhound",
  ];
  const versions = ["/docs", "/docs/next"];

  test("passes the files markdownFiles writes", () => {
    expect(checkOutput(written(files), "/docs/", versions, routes)).toEqual([]);
  });

  test("names a page with no twin, one its section's llms.txt lacks, and one its llms-full.txt lacks", () => {
    const broken = new Map(files);
    broken.delete("/docs/foxhound.md");
    broken.set(
      "/docs/next/collie/llms.txt",
      broken.get("/docs/next/collie/llms.txt")?.replace("/docs/next/collie/install.md", "/x.md") ??
        "",
    );
    broken.set("/docs/collie/llms-full.txt", broken.get("/docs/collie.md") ?? "");
    expect(checkOutput(written(broken), "/docs/", versions, routes)).toEqual([
      "/docs/collie/install: /docs/collie/llms-full.txt lacks the text of /docs/collie/install.md",
      "/docs/foxhound: its twin /docs/foxhound.md is missing",
      "/docs/next/collie/install: /docs/next/collie/llms.txt does not list /docs/next/collie/install.md",
    ]);
  });

  test("names a page listed only in the root's files rather than its section's", () => {
    const moved = new Map(files);
    moved.set("/docs/foxhound/llms.txt", "# Goodfetch Foxhound\n");
    moved.set(
      "/docs/llms.txt",
      `${moved.get("/docs/llms.txt") ?? ""}- [Home](/docs/foxhound.md)\n`,
    );
    expect(checkOutput(written(moved), "/docs/", versions, routes)).toContain(
      "/docs/foxhound: /docs/foxhound/llms.txt does not list /docs/foxhound.md",
    );
  });

  test("names a root llms.txt that does not list a section's files", () => {
    const unlisted = new Map(files);
    unlisted.set(
      "/docs/next/llms.txt",
      unlisted.get("/docs/next/llms.txt")?.replace("(/docs/next/foxhound/llms-full.txt)", "") ?? "",
    );
    expect(checkOutput(written(unlisted), "/docs/", versions, routes)).toEqual([
      "/docs/next/llms.txt does not list /docs/next/foxhound/llms-full.txt",
    ]);
  });
});
