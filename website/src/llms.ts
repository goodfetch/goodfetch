// The Markdown the docs site serves beside its HTML (ADRs 0030 and 0097):
// each page's `.md` twin, each Product section's `llms.txt` and
// `llms-full.txt`, and the root's pair, for every kept version; and the
// build-output check that every page has its twin, its entry and its text in
// its own section's files, or the root's for a root page (ADR 0068).
//
// A twin sits at its page's URL with `.md`, or at `index.md` under a URL that
// ends with `/`. Its links to other pages point at their twins in the same
// version, from the site's root, so they read the same in `llms-full.txt`
// and on any host that serves the site under `/docs/`.
import { existsSync, readFileSync } from "node:fs";
import { join, posix } from "node:path";
import { productName, products } from "./product-switch.ts";

export interface Page {
  /** The page's URL path, base URL and version path included: `/docs/next/collie/install`. */
  permalink: string;
  /** The page's Markdown file. */
  source: string;
  title: string;
  description: string;
  /** The file's text, front matter included. */
  text: string;
}

export interface Version {
  /** The version's URL path: `/docs` for stable, `/docs/next` for `next`. */
  path: string;
  pages: Page[];
}

export interface Site {
  title: string;
  description: string;
}

/** A URL path without its trailing `/`. */
function bare(path: string): string {
  return path.endsWith("/") ? path.slice(0, -1) : path;
}

/** The URL of a page's `.md` twin. */
export function twinOf(permalink: string): string {
  return permalink.endsWith("/") ? `${permalink}index.md` : `${permalink}.md`;
}

/** The Product section a URL belongs to in the version at `versionPath`, or none for a root page. */
function sectionOf(versionPath: string, url: string): string | undefined {
  const [first] = url.slice(bare(versionPath).length).replace(/^\//, "").split("/");
  return products.find((product) => product === first);
}

/** The folder of the llms files that list a page of `section`, or of the root. */
function llmsFolder(versionPath: string, section: string | undefined): string {
  return section === undefined ? bare(versionPath) : `${bare(versionPath)}/${section}`;
}

const scheme = /^[a-z][a-z\d+.-]*:/i;
const fence = /^ {0,3}(`{3,}|~{3,})/;

/** Points the links of `text`, outside fenced blocks, that name a page's file at its twin. */
function retarget(text: string, source: string, twins: ReadonlyMap<string, string>): string {
  let open: string | undefined;
  return text
    .split("\n")
    .map((line) => {
      const marker = fence.exec(line)?.[1];
      if (marker !== undefined && (open === undefined || marker.startsWith(open))) {
        open = open === undefined ? marker : undefined;
        return line;
      }
      if (open !== undefined) {
        return line;
      }
      return line.replace(/\]\(([^)\s]+)\)/g, (link, target: string) => {
        if (scheme.test(target) || target.startsWith("/") || target.startsWith("#")) {
          return link;
        }
        const [file = "", fragment] = target.split("#", 2);
        const twin = twins.get(posix.normalize(posix.join(posix.dirname(source), file)));
        if (twin === undefined) {
          return link;
        }
        return `](${twin}${fragment === undefined ? "" : `#${fragment}`})`;
      });
    })
    .join("\n");
}

/** A page's twin: its text without front matter, under its title when it has no heading. */
function twinText(page: Page, twins: ReadonlyMap<string, string>): string {
  const body = page.text.replace(/^---\r?\n[\s\S]*?\r?\n---\r?\n/, "").replace(/^\s+/, "");
  const titled = body.startsWith("# ") ? body : `# ${page.title}\n\n${body}`;
  return retarget(titled, page.source, twins);
}

function entry(page: Page): string {
  return `- [${page.title}](${twinOf(page.permalink)})${page.description === "" ? "" : `: ${page.description}`}`;
}

function llmsIndex(
  title: string,
  summary: string | undefined,
  parts: [string, string[]][],
): string {
  const lines = [`# ${title}`, ""];
  if (summary !== undefined && summary !== "") {
    lines.push(`> ${summary}`, "");
  }
  for (const [heading, entries] of parts) {
    if (entries.length > 0) {
      lines.push(`## ${heading}`, "", ...entries, "");
    }
  }
  return lines.join("\n");
}

/** Every twin and llms file of `versions`, by URL path. */
export function markdownFiles(versions: readonly Version[], site: Site): Map<string, string> {
  const files = new Map<string, string>();
  for (const version of versions) {
    const twins = new Map(version.pages.map((page) => [page.source, twinOf(page.permalink)]));
    const pages = version.pages.toSorted((a, b) => a.permalink.localeCompare(b.permalink));
    for (const page of pages) {
      files.set(twinOf(page.permalink), twinText(page, twins));
    }
    const fullText = (listed: Page[]) =>
      listed.map((page) => files.get(twinOf(page.permalink)) ?? "").join("\n");
    const root = pages.filter((page) => sectionOf(version.path, page.permalink) === undefined);
    for (const product of products) {
      const folder = llmsFolder(version.path, product);
      const own = pages.filter((page) => sectionOf(version.path, page.permalink) === product);
      const home = own.find((page) => bare(page.permalink) === folder);
      files.set(
        `${folder}/llms.txt`,
        llmsIndex(productName(product), home?.description, [["Docs", own.map(entry)]]),
      );
      files.set(`${folder}/llms-full.txt`, fullText(own));
    }
    const folder = llmsFolder(version.path, undefined);
    files.set(
      `${folder}/llms.txt`,
      llmsIndex(site.title, site.description, [
        [
          "Products",
          products.map(
            (product) =>
              `- [${productName(product)}](${folder}/${product}/llms.txt): the section's pages; their text is in [llms-full.txt](${folder}/${product}/llms-full.txt)`,
          ),
        ],
        ["Docs", root.map(entry)],
      ]),
    );
    files.set(`${folder}/llms-full.txt`, fullText(root));
  }
  return files;
}

/**
 * What the built site at `outDir` lacks: for each page at `routes`, its twin,
 * its entry in its own section's `llms.txt`, or the root's for a root page,
 * and its twin's text in the matching `llms-full.txt`; and, for each version
 * at `versionPaths`, the root `llms.txt`'s links to each section's files.
 */
export function checkOutput(
  outDir: string,
  baseUrl: string,
  versionPaths: readonly string[],
  routes: readonly string[],
): string[] {
  const read = (url: string): string | undefined => {
    const file = join(outDir, url.slice(baseUrl.length));
    return existsSync(file) ? readFileSync(file, "utf8") : undefined;
  };
  const versionOf = (route: string): string =>
    versionPaths
      .map(bare)
      .filter((path) => route === path || route.startsWith(`${path}/`))
      .reduce((longest, path) => (path.length > longest.length ? path : longest), bare(baseUrl));
  const problems: string[] = [];
  for (const route of routes) {
    const version = versionOf(route);
    const folder = llmsFolder(version, sectionOf(version, route));
    const twin = twinOf(route);
    const text = read(twin);
    if (text === undefined) {
      problems.push(`${route}: its twin ${twin} is missing`);
      continue;
    }
    if (!(read(`${folder}/llms.txt`) ?? "").includes(`](${twin})`)) {
      problems.push(`${route}: ${folder}/llms.txt does not list ${twin}`);
    }
    if (!(read(`${folder}/llms-full.txt`) ?? "").includes(text.trim())) {
      problems.push(`${route}: ${folder}/llms-full.txt lacks the text of ${twin}`);
    }
  }
  for (const version of versionPaths.map(bare)) {
    const index = read(`${version}/llms.txt`) ?? "";
    for (const product of products) {
      for (const file of ["llms.txt", "llms-full.txt"]) {
        const url = `${version}/${product}/${file}`;
        if (!index.includes(`](${url})`)) {
          problems.push(`${version}/llms.txt does not list ${url}`);
        }
      }
    }
  }
  return problems.toSorted();
}
