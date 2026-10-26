// The SQL tab with a result (ADR 0073's `visual` screens; spec #593), in
// the pinned Playwright image only (TESTING.md, Visual regression): its
// statement, its runs, and a page of rows of mixed directions, the rest of
// the result counted.
import { ConsoleI18n } from "@goodfetch/console/i18n";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { SqlTabView } from "#sql-tab/sql-tab.tsx";
import "@goodfetch/console/styles.css";

test("the SQL tab with a result", { tags: ["story:12-210"] }, async () => {
  await render(
    <ConsoleI18n>
      <main className="bg-background p-6 text-foreground">
        <SqlTabView
          runId="shop-monday"
          ran={{
            kind: "answered",
            result: {
              columns: [
                { name: "title", type: "VARCHAR" },
                { name: "runs", type: "BIGINT" },
              ],
              rows: [
                ["Café 1", 2],
                ["كتاب ٢", 1],
                ["שלום 3", 1],
              ],
              total: 120,
            },
          }}
          page={0}
          onPage={() => {}}
          onRun={() => {}}
        />
      </main>
    </ConsoleI18n>,
  );
  await expect.element(page.getByRole("table")).toBeVisible();

  await expect.element(page.getByRole("main")).toMatchScreenshot("sql-tab-answered");
});
