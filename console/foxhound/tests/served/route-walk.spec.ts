// The route walk (spec #593, story 202; ADR 0094; TESTING.md, "Which
// Product's executable a test runs", containment check 3): on Foxhound's
// executable built from the commit, with a Daemon and home of its own on
// port 0, every route of Collie's build, read from Collie's route data,
// renders in Foxhound's build: the executable serves its page, and the
// shell shows the route's screen, which names it in the page's title
// beside Goodfetch Foxhound. A route's parameter takes a value of its own.
import { readFile } from "node:fs/promises";
import { expect, test } from "@goodfetch/test-support/playwright";

/** Collie's route data, which its package commits for the inventory and its executable. */
const COLLIE_ROUTES = new URL("../../../collie/routes.json", import.meta.url);

/** The routes of Collie's build. */
async function collieRoutes(): Promise<string[]> {
  const listed: unknown = JSON.parse(await readFile(COLLIE_ROUTES, "utf8"));
  if (
    typeof listed !== "object" ||
    listed === null ||
    !("routes" in listed) ||
    !Array.isArray(listed.routes) ||
    !listed.routes.every((route) => typeof route === "string")
  ) {
    throw new Error(`${COLLIE_ROUTES.pathname} is no route list`);
  }
  return listed.routes;
}

/** The path that opens `route`, each parameter given a value. */
function pathOf(route: string): string {
  return route
    .split("/")
    .map((segment) => (segment.startsWith("$") ? `walked-${segment.slice(1)}` : segment))
    .join("/");
}

test.use({ product: "foxhound" });

test(
  "every route of Collie's build renders in Foxhound's build on Foxhound's executable",
  { annotation: [{ type: "covers", description: "story:12-202" }] },
  async ({ page, daemon, signIn }) => {
    const routes = await collieRoutes();
    expect(routes.length).toBeGreaterThan(0);
    await signIn.withLink(page);

    for (const route of routes) {
      await test.step(route, async () => {
        const response = await page.goto(new URL(pathOf(route), daemon.url).href);
        expect(response?.status(), `${route} on Foxhound's executable`).toBe(200);
        await expect(page).toHaveTitle(/^.+ · Goodfetch Foxhound$/);
        await expect(page.getByRole("navigation", { name: "Console" })).toBeVisible();
        await expect(page.getByRole("main")).not.toBeEmpty();
      });
    }
  },
);
