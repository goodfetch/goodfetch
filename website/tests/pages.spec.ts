// The built docs site in a browser (ADRs 0068, 0073 and 0097; TESTING.md,
// Docs site): every page of every kept version loads, `next`'s pages pass
// axe in both themes, every footer links the accessibility statement, and
// the header's switcher, the version switcher, the copy buttons and the
// section-scoped search work.
import { expect, expectAccessible, test } from "@goodfetch/test-support/playwright";
import type { Locator, Page } from "@playwright/test";
import { keptVersions, nextPages, serveSite, sitemapPages, type Site } from "./site.ts";

let site: Site;

test.beforeAll(async () => {
  site = await serveSite();
});

test.afterAll(async () => {
  await site.close();
});

/** Opens `path` under the base URL, and waits until React has hydrated it. */
async function open(page: Page, path: string): Promise<void> {
  const response = await page.goto(site.at(path));
  expect(response?.status(), `${path} answers`).toBe(200);
  await expect(page.locator("html")).toHaveAttribute("data-has-hydrated", "true");
}

test(
  "every page of every kept version loads with no console, hydration or request error",
  { annotation: { type: "covers", description: "story:1-48" } },
  async ({ page }) => {
    const pages = sitemapPages();
    expect(pages).toContain("collie/install");
    for (const path of pages) {
      await test.step(path, () => open(page, path));
    }
  },
);

for (const colorScheme of ["light", "dark"] as const) {
  test.describe(`in the ${colorScheme} theme`, () => {
    test.use({ colorScheme });

    test(
      "every page of next passes axe",
      { annotation: { type: "covers", description: "story:1-116" } },
      async ({ page }) => {
        for (const path of nextPages(sitemapPages())) {
          await test.step(path, async () => {
            await open(page, path);
            await expect(page.locator("html")).toHaveAttribute("data-theme", colorScheme);
            await expectAccessible(page);
          });
        }
      },
    );
  });
}

test(
  "the accessibility statement is served and every page's footer links it",
  { annotation: { type: "covers", description: "story:1-116" } },
  async ({ page }) => {
    await open(page, "accessibility");
    await expect(page.getByRole("heading", { level: 1 })).toHaveText("Accessibility statement");
    for (const path of sitemapPages()) {
      await test.step(path, async () => {
        await open(page, path);
        await expect(
          page.getByRole("contentinfo").getByRole("link", { name: "Accessibility statement" }),
        ).toHaveAttribute("href", "/docs/accessibility");
      });
    }
  },
);

test.describe(
  "the header's switcher between the Products",
  { annotation: { type: "covers", description: "story:1-119" } },
  () => {
    test("keeps the page when the other section has it", async ({ page }) => {
      await open(page, "collie/runs");

      await page
        .getByRole("navigation", { name: "Product" })
        .getByRole("link", { name: "Foxhound" })
        .click();

      await expect(page).toHaveURL(site.at("foxhound/runs"));
      await expect(page.getByRole("heading", { level: 1 })).toHaveText("Runs");
    });

    test("leads to the other section's home when it lacks the page", async ({ page }) => {
      await open(page, "collie/install");

      await page
        .getByRole("navigation", { name: "Product" })
        .getByRole("link", { name: "Foxhound" })
        .click();

      await expect(page).toHaveURL(site.at("foxhound"));
    });
  },
);

test(
  "the version switcher offers every kept version at the page being read",
  { annotation: { type: "covers", description: "story:1-39" } },
  async ({ page }) => {
    await open(page, "collie/install");

    const navigation = page.getByRole("navigation", { name: "Main" });
    for (const label of keptVersions()) {
      await expect(
        navigation.getByRole("link", { name: label, exact: true, includeHidden: true }).last(),
      ).toHaveAttribute("href", /\/collie\/install$/);
    }
  },
);

test.describe("a code block's copy button", () => {
  // Chromium asks the page for the clipboard permission, which Firefox and
  // WebKit grant a click without being asked.
  test.use({
    permissions: async ({ browserName }, use) => {
      await use(browserName === "chromium" ? ["clipboard-read", "clipboard-write"] : []);
    },
  });

  test("copies its code", async ({ page }) => {
    await open(page, "collie/install");

    await page.getByRole("button", { name: "Copy code to clipboard" }).first().click();

    await expect(page.getByRole("button", { name: "Copied" })).toBeVisible();
  });
});

/** Opens `path` and types `query` in the header's search box. */
async function search(page: Page, path: string, query: string): Promise<Locator> {
  await open(page, path);
  const box = page.getByRole("navigation", { name: "Main" }).getByLabel("Search", { exact: true });
  await box.click();
  await box.pressSequentially(query);
  return box;
}

test.describe("search", { annotation: { type: "covers", description: "story:1-119" } }, () => {
  test("from a section finds its pages, not the other section's", async ({ page }) => {
    // Both sections have a page titled "Runs": the result is Collie's.
    const box = await search(page, "collie/runs", "Checkpoint");
    await expect(page.getByRole("option").first()).toBeVisible();
    await expect(page.getByRole("link", { name: /^See all results within/ })).toHaveAttribute(
      "href",
      /[?&]ctx=collie(&|$)/,
    );

    await box.press("Enter");

    await expect(page).toHaveURL(/\/docs\/collie\/runs(\?|#|$)/);
  });

  test("from a section also finds the root pages", async ({ page }) => {
    const box = await search(page, "collie/install", "Choose a Product");
    await expect(page.getByRole("option").first()).toBeVisible();

    await box.press("Enter");

    await expect(page).toHaveURL(/\/docs\/(\?|#|$)/);
  });

  test("from the root finds no section's page", async ({ page }) => {
    await search(page, "", "Checkpoint");

    await expect(page.getByRole("option")).toHaveText(["No results"]);
  });
});
