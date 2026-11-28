// The built docs site, as its tests read it: served under `/docs/` on a
// loopback port 0 (TESTING.md, Docs site), with the pages its `sitemap.xml`
// lists and the versions it keeps.
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { serveStatic, type StaticServer } from "@goodfetch/test-support/serve-static";
import { readVersions, versionsConfig } from "../src/versions.ts";

const website = join(import.meta.dirname, "..");

/** The folder the site is built into. */
export const built: string = join(website, "../target/docs-site/build");

/** The base URL the site is served under, as on `goodfetch.dev`. */
export const base = "/docs/";

/** The built site, served until `close`. */
export interface Site extends StaticServer {
  /** The absolute URL of `path`, a path under the base URL: `collie/install`. */
  at(path: string): string;
}

/** Serves the built site, which the `docs-site-build` setup project wrote. */
export async function serveSite(): Promise<Site> {
  if (!existsSync(join(built, "sitemap.xml"))) {
    throw new Error(`${built} holds no built site; the docs-site-build project builds it`);
  }
  const server = await serveStatic(built, base);
  return { ...server, at: (path) => new URL(path, server.url).href };
}

/** Every page's path under the base URL, as `sitemap.xml` lists it, every kept version's. */
export function sitemapPages(): string[] {
  const sitemap = readFileSync(join(built, "sitemap.xml"), "utf8");
  return [...sitemap.matchAll(/<loc>https:\/\/goodfetch\.dev\/docs\/([^<]*)<\/loc>/g)]
    .map(([, path = ""]) => path)
    .toSorted((first, second) => first.localeCompare(second));
}

/** The kept versions' labels, newest first: `next` alone before the first release. */
export function keptVersions(): string[] {
  const config = versionsConfig(readVersions(join(website, "versions.json")));
  return Object.values(config.versions).map((version) => version.label);
}

/**
 * The pages of `next` in `pages`: all of them before the first release,
 * when `next` is the one version and sits at the root; those under `next/`
 * after it, when stable holds the root.
 */
export function nextPages(pages: readonly string[]): string[] {
  if (keptVersions().length === 1) {
    return [...pages];
  }
  return pages.filter((path) => path === "next" || path.startsWith("next/"));
}
