// The docs site's plugin that, after the build, writes each page's `.md`
// twin and the llms files of each kept version (`llms.ts`), then fails the
// build when a page lacks its twin, its entry or its text (ADRs 0068 and
// 0097). It reads the pages from the docs plugin's content and the Markdown
// pages from the pages plugin's; a Markdown page counts as a root page of the
// stable version, the one at the site's base URL.
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import type { Plugin } from "@docusaurus/types";
import { type Page, type Version, checkOutput, markdownFiles } from "./llms.ts";

interface DocsContent {
  loadedVersions: {
    path: string;
    docs: { permalink: string; source: string; title: string; description: string }[];
  }[];
}

interface MarkdownPage {
  type: "mdx";
  permalink: string;
  source: string;
  title?: string;
  description?: string;
}

function isDocsContent(content: unknown): content is DocsContent {
  return (
    typeof content === "object" &&
    content !== null &&
    Array.isArray((content as Partial<DocsContent>).loadedVersions)
  );
}

function isMarkdownPage(page: unknown): page is MarkdownPage {
  return typeof page === "object" && page !== null && (page as { type?: unknown }).type === "mdx";
}

/** The routes a 404 page or a search page answers, which have no twin. */
function hasTwin(route: string, baseUrl: string): boolean {
  return route !== `${baseUrl}404.html` && route !== `${baseUrl}search`;
}

export default function llmsPlugin(): Plugin {
  return {
    name: "goodfetch-llms",
    postBuild({ outDir, plugins, routesPaths, siteConfig, siteDir }) {
      const contentOf = (name: string): unknown =>
        plugins.find((plugin) => plugin.name === name)?.content;
      const docs = contentOf("docusaurus-plugin-content-docs");
      if (!isDocsContent(docs)) {
        throw new Error("the docs plugin's content has no versions");
      }
      const file = (source: string) => resolve(siteDir, source.replace(/^@site\//, ""));
      // A section's home has a permalink that ends with `/`; the site, whose
      // `trailingSlash` is false, serves it without, except at the base URL.
      const route = (permalink: string) =>
        permalink === siteConfig.baseUrl ? permalink : permalink.replace(/\/$/, "");
      const page = (
        permalink: string,
        source: string,
        title: string,
        description: string,
      ): Page => ({
        permalink: route(permalink),
        source: file(source),
        title,
        description,
        text: readFileSync(file(source), "utf8"),
      });
      const baseUrl = siteConfig.baseUrl;
      const markdownPages = [contentOf("docusaurus-plugin-content-pages")]
        .flat()
        .filter(isMarkdownPage)
        .map((each) => page(each.permalink, each.source, each.title ?? "", each.description ?? ""));
      const versions: Version[] = docs.loadedVersions.map((version) => ({
        path: version.path,
        pages: [
          ...version.docs.map((doc) => page(doc.permalink, doc.source, doc.title, doc.description)),
          ...(`${version.path.replace(/\/$/, "")}/` === baseUrl ? markdownPages : []),
        ],
      }));
      const files = markdownFiles(versions, {
        title: siteConfig.title,
        description: siteConfig.tagline,
      });
      for (const [url, text] of files) {
        const target = join(outDir, url.slice(baseUrl.length));
        mkdirSync(dirname(target), { recursive: true });
        writeFileSync(target, text);
      }
      const problems = checkOutput(
        outDir,
        baseUrl,
        versions.map((version) => version.path),
        routesPaths.filter((each) => hasTwin(each, baseUrl)),
      );
      if (problems.length > 0) {
        throw new Error(`the built site's Markdown is incomplete:\n${problems.join("\n")}`);
      }
    },
  };
}
