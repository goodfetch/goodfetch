// The built docs site's links and anchors, checked offline by the pinned
// lychee (TESTING.md, Docs site): every link from one page to another, and
// every `#fragment` it names, leads somewhere. External links are the
// `docs-external-links` Live check's (`live/`), not this test's: lychee's
// offline mode leaves them out.
import { spawnSync } from "node:child_process";
// Playwright's own `test`: test support's opens a browser context for its
// page checks, and this test opens no page.
import { expect, test } from "@playwright/test";
import { lychee, withSiteCopy } from "./lychee.ts";

test(
  "every link and anchor between the site's pages resolves",
  { annotation: { type: "covers", description: "story:1-48" } },
  () => {
    withSiteCopy((root) => {
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
    });
  },
);
