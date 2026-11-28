// The Live check `docs-external-links` (ADRs 0064 and 0068; TESTING.md,
// Docs site): every link from the built site to another site answers. It
// runs before each release candidate while the nightly tier is off, blocks
// nothing directly and reports to the results store. A broken link fails it;
// an other side that did not answer, or answered as busy or failing, makes
// it inconclusive, which Playwright reports as skipped. Links to
// `goodfetch.dev` are the site's own, which the link check tests offline and
// `docs-deploy-smoke` online.
import { spawnSync } from "node:child_process";
// Playwright's own `test`: this check opens no page.
import { expect, test } from "@playwright/test";
import { lychee, withSiteCopy } from "../lychee.ts";
import { sortFailures } from "./lychee-report.ts";

test("docs-external-links", () => {
  test.setTimeout(300_000);
  const sorted = withSiteCopy((root) => {
    // lychee reads GITHUB_TOKEN from the environment for github.com links,
    // so a link into a private repository answers to the job's token.
    const run = spawnSync(
      lychee(),
      [
        "--no-progress",
        "--format",
        "json",
        "--root-dir",
        root,
        "--scheme",
        "https",
        "--scheme",
        "http",
        "--exclude",
        String.raw`^https?://goodfetch\.dev(/|$)`,
        "--max-retries",
        "2",
        "docs/**/*.html",
      ],
      { cwd: root, encoding: "utf8", maxBuffer: 64 * 1024 * 1024 },
    );
    expect(run.error).toBeUndefined();
    expect([0, 2], `lychee's errors:\n${run.stderr}`).toContain(run.status);
    return sortFailures(run.stdout);
  });
  expect(sorted.broken, "links whose site answers that they are missing").toEqual([]);
  test.skip(
    sorted.unavailable.length > 0,
    `inconclusive: no answer from\n${sorted.unavailable.join("\n")}`,
  );
});
