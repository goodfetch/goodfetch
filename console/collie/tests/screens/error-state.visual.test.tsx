// An error state (ADR 0073's `visual` screens): a run's page whose run the
// engine cannot read, its failure shown with the catalog's message and every
// field the engine sent, against the in-memory adapter of the protocol, in
// the pinned Playwright image only (TESTING.md, Visual regression). The
// failure carries no point in time the page shows, so no clock needs fixing.
import { createProtocolAdapter, failure } from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { createConsoleClient, type PageVisibility } from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";
import "#styles.css";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

test("a run's page whose run the engine cannot read", async () => {
  const adapter = createProtocolAdapter();
  adapter.answer("needs_you.list", { items: [] });
  adapter.answer("run.list", { runs: [] });
  adapter.answer("run.usage", {
    browser_time: 0,
    model: { calls: 0, tokens: 0 },
    proxy_bytes: 0,
    requests: 0,
  });
  adapter.answer(
    "run.progress",
    failure("run_state_unavailable", {
      operation: "run.progress",
      observed: "run.sqlite is locked by another program",
      limit: { key: "timeout.run_state", value: 5_000, layer: "user_configuration" },
      fix: "close the program that holds run.sqlite, then open the run again",
      call_log: ["opening runs/nightly/run.sqlite", "run.sqlite is locked", "gave up after 5 s"],
      path: "runs/nightly/run.sqlite",
    }),
  );
  const client = createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "collie",
  });
  await render(
    <ConsoleI18n>
      <Shell client={client} route="/runs/nightly" />
    </ConsoleI18n>,
  );
  await expect.element(page.getByRole("alert")).toBeVisible();

  await expect.element(page.getByRole("main")).toMatchScreenshot("error-state-run-page");
  client.close();
});
