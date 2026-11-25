// Builds the docs site once per Playwright run, before any test of the
// `docs-site` projects starts (TESTING.md, Docs site): the render of
// `docs/`, then every kept version, into `target/docs-site/build/`. A build
// that fails, on a broken link or a page without its `.md` twin, fails this
// setup, and the site's tests do not run.
import { execFileSync } from "node:child_process";
import { test as setup } from "@playwright/test";

setup("build the docs site", () => {
  setup.setTimeout(300_000);
  execFileSync("pnpm", ["--filter", "@goodfetch/website", "run", "build"], {
    cwd: `${import.meta.dirname}/../..`,
    stdio: "inherit",
  });
});
