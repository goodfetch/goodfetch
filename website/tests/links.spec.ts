// The built docs site's links and anchors, checked offline by the pinned
// lychee (TESTING.md, Docs site): every link from one page to another, and
// every `#fragment` it names, leads somewhere. The site is laid out under a
// `docs/` folder, so that its absolute `/docs/...` links resolve as on
// `goodfetch.dev`. External links are the `docs-external-links` Live
// check's, not this test's: lychee's offline mode leaves them out.
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { homedir, tmpdir } from "node:os";
import { join } from "node:path";
// Playwright's own `test`: test support's opens a browser context for its
// page checks, and this test opens no page.
import { expect, test } from "@playwright/test";
import { built } from "./site.ts";

const pins = join(import.meta.dirname, "../../test-support/lychee/pins.toml");

/** The tests' copy of the pinned lychee, which `ci/lychee-fetch.sh` writes. */
function lychee(): string {
  const version = /^version = "([^"]+)"$/m.exec(readFileSync(pins, "utf8"))?.[1];
  if (version === undefined) {
    throw new Error(`${pins} pins no lychee version`);
  }
  const downloads =
    process.env["GOODFETCH_LYCHEE_DOWNLOADS"] ?? join(homedir(), ".cache/goodfetch/lychee");
  const executable = join(
    downloads,
    version,
    process.platform === "win32" ? "lychee.exe" : "lychee",
  );
  if (!existsSync(executable)) {
    throw new Error(`${executable} is missing: run ci/lychee-fetch.sh (part of \`just fetch\`)`);
  }
  return executable;
}

test(
  "every link and anchor between the site's pages resolves",
  { annotation: { type: "covers", description: "story:1-48" } },
  () => {
    const root = mkdtempSync(join(tmpdir(), "goodfetch-docs-links-"));
    try {
      cpSync(built, join(root, "docs"), { recursive: true });
      // `index.html,.` and the `html` fallback resolve a link as the site's
      // host serves it with `trailingSlash: false`: `/docs/collie` is
      // `collie.html`, `/docs/` is `index.html`.
      const run = spawnSync(
        lychee(),
        [
          "--offline",
          "--no-progress",
          "--include-fragments",
          "--root-dir",
          root,
          "--index-files",
          "index.html,.",
          "--fallback-extensions",
          "html",
          "--format",
          "compact",
          "docs/**/*.html",
        ],
        { cwd: root, encoding: "utf8" },
      );
      expect(run.error).toBeUndefined();
      expect(run.status, `lychee's report:\n${run.stdout}${run.stderr}`).toBe(0);
    } finally {
      rmSync(root, { recursive: true, force: true });
    }
  },
);
