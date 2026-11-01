// Foxhound's screens on its target (ADRs 0087 and 0096): Foxhound's build
// shows a run's Foxhound views, each answered by Foxhound's engine, only for
// a run on Foxhound's engine; a run that gets Collie shows only the views
// every run has. Both runs are started as the Console starts them, so a
// gateway puts each on the engine of the Product its start names. A target
// serving Collie's build carries no Foxhound screen and skips the case.
import { expect, test } from "@goodfetch/test-support/conformance";
import { asConsole, cookieHeader, ProgramConnection } from "./target.ts";

/** A crawl with one seed it never fetches. */
const CRAWL = { seeds: [{ url: "ftp://example.invalid/", label: "start" }] };

test(
  "foxhounds-screens-show-only-for-a-run-on-foxhounds-engine",
  {
    annotation: [
      { type: "capability", description: "foxhound-screens" },
      { type: "covers", description: "story:12-231" },
      { type: "covers", description: "story:12-203" },
    ],
  },
  async ({ page, context, daemon, signIn }) => {
    test.skip(daemon.product !== "foxhound", "only Foxhound's build carries Foxhound's screens");
    await signIn.withLink(page);
    // A Daemon takes the page's cookie, a gateway its own token.
    const console = await ProgramConnection.open(
      daemon,
      asConsole(daemon),
      daemon.gateway === undefined ? await cookieHeader(context, daemon.url) : undefined,
    );
    let plain: string;
    let crawl: string;
    try {
      plain = await console.start("run.start", {});
      crawl = await console.start("crawl.start", CRAWL);
    } finally {
      console.close();
    }

    await page.goto(new URL(`/runs/${plain}`, daemon.url).href);
    await expect(page.getByRole("region", { name: "How it stands" })).toBeVisible();
    await expect(page.getByRole("tablist")).toHaveCount(0);

    await page.goto(new URL(`/runs/${crawl}`, daemon.url).href);
    const tabs = page.getByRole("tablist", { name: "Run views" }).getByRole("tab");
    await expect(tabs.filter({ hasText: "Seeds" })).toHaveCount(1);
    await page.getByRole("tab", { name: "Seeds", exact: true }).click();
    const seeds = page.getByRole("tabpanel", { name: "Seeds" });
    await expect(seeds).toContainText("ftp://example.invalid/");
    await expect(seeds).not.toContainText("not_in_product");
  },
);
