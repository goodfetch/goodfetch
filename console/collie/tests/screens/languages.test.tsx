// The Runs screen for a person reading another language (ADR 0073), against
// the in-memory adapter of the protocol: every value the engine sent (Run
// IDs, tags, a Recipe, a status line, points in time) renders inside
// `translate="no"`, so browser translation leaves it alone, each point in
// time with its machine form beside it, while the screen's own words stay
// translatable. Formatted text is asserted in the `unit` project only
// (TESTING.md, Localization); the pseudo-locale projects check every story.
import type { RunIndexRow } from "@goodfetch/collie-client/generated";
import { createProtocolAdapter } from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { createConsoleClient, type PageVisibility } from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

const RUN: RunIndexRow = {
  run_id: "nightly-prices",
  location: "runs/nightly-prices",
  kind: "collie",
  needs: "collie",
  tags: ["team:shop", "eu", "goodfetch.recipe:shop-prices"],
  lifecycle: "finished",
  verdict: "verified",
  completeness: "complete",
  started_at: "2026-10-31T23:00:00Z",
  ended_at: "2026-11-01T00:15:00Z",
  progress_message: "Read 40 of 40 pages",
  presence: "present",
  replayed: false,
};

/** Whether `element` sits inside `translate="no"`. */
function untranslated(element: Element): boolean {
  return element.closest('[translate="no"]') !== null;
}

test(
  "the Runs screen renders every value of the engine inside translate=no, its points in time in machine form",
  { tags: ["story:12-132", "screen:/runs"] },
  async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("needs_you.list", { items: [] });
    adapter.answer("run.list", { runs: [RUN] });
    const client = createConsoleClient({
      transport: adapter.transport,
      bodyFetcher: adapter.bodyFetcher,
      visibility: visible,
      product: "collie",
    });
    await render(
      <ConsoleI18n>
        <Shell client={client} route="/runs" />
      </ConsoleI18n>,
    );
    const row = page.getByRole("row").filter({ hasText: "nightly-prices" });
    await expect.element(row).toBeVisible();

    const values = ["nightly-prices", "shop-prices", "Read 40 of 40 pages"].map((text) =>
      row.getByText(text, { exact: true }).element(),
    );
    const times = row.element().querySelectorAll("time");
    expect({
      values: values.map(untranslated),
      tags: untranslated(row.getByText(/team:shop/u).element()),
      times: [...times].map((time) => [time.dateTime, untranslated(time)]),
      lifecycle: untranslated(row.getByRole("cell", { name: "finished", exact: true }).element()),
    }).toEqual({
      values: [true, true, true],
      tags: true,
      times: [
        ["2026-10-31T23:00:00Z", true],
        ["2026-11-01T00:15:00Z", true],
      ],
      lifecycle: false,
    });
    client.close();
  },
);
