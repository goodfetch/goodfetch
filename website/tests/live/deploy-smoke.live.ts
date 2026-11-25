// The Live check `docs-deploy-smoke` (ADRs 0064, 0068 and 0097; TESTING.md,
// Docs site): after each production deploy, the deployed site answers
// through `goodfetch.dev/docs` (`GOODFETCH_DOCS_URL` names another) with
// the home page, a page of each kept version, and a page, its `.md` twin,
// `llms.txt` and `llms-full.txt` of each Product's section and of the root,
// each with its content type. It blocks nothing directly and reports to the
// results store. While no site answers there, it is inconclusive, which
// Playwright reports as skipped.
import { join } from "node:path";
// Playwright's own `test`: this check opens no page.
import { expect, test } from "@playwright/test";
import { readVersions, versionsConfig } from "../../src/versions.ts";

const base = (process.env["GOODFETCH_DOCS_URL"] ?? "https://goodfetch.dev/docs").replace(/\/$/, "");

const html = "text/html";
const markdown = "text/markdown";

/** The paths under the base URL to load, each with the content type it must have. */
function expected(): [string, string][] {
  const config = versionsConfig(readVersions(join(import.meta.dirname, "../../versions.json")));
  const versions = config.onlyIncludeVersions.map((version): [string, string] => {
    const path = version === config.lastVersion ? "" : config.versions[version]?.path;
    if (path === undefined) {
      throw new Error(`the kept version ${version} has no path`);
    }
    return [`${path === "" ? "" : `${path}/`}collie/install`, html];
  });
  const sections = ["collie", "foxhound"].flatMap((section): [string, string][] => [
    [section, html],
    [`${section}.md`, markdown],
    [`${section}/llms.txt`, markdown],
    [`${section}/llms-full.txt`, markdown],
  ]);
  return [
    ["", html],
    ...versions,
    ...sections,
    ["accessibility", html],
    ["index.md", markdown],
    ["llms.txt", markdown],
    ["llms-full.txt", markdown],
  ];
}

test("docs-deploy-smoke", async () => {
  try {
    await fetch(`${base}/`, { method: "HEAD" });
  } catch (error) {
    test.skip(true, `inconclusive: no site answers at ${base}/ (${String(error)})`);
  }
  const answers = await Promise.all(
    expected().map(async ([path, type]) => {
      const url = `${base}/${path}`;
      return { url, type, response: await fetch(url) };
    }),
  );
  for (const { url, type, response } of answers) {
    expect.soft(response.status, url).toBe(200);
    expect
      .soft(response.headers.get("content-type") ?? "", url)
      .toMatch(new RegExp(`^${type}(;|$)`));
  }
});
