// Foxhound's views of a run that needs Foxhound (spec #593, story 203),
// against the in-memory adapter of Foxhound's engine: each reads its
// operation of Foxhound's catalog for the run and shows what it answers,
// or the engine's refusal as the Console shows any.
import type { ConsoleClient, PageVisibility } from "@goodfetch/console/client";
import { createConsoleClient } from "@goodfetch/console/client";
import type { RunView } from "@goodfetch/console/console-build";
import { ConsoleI18n } from "@goodfetch/console/i18n";
import type { CrawlRequestOutcome } from "@goodfetch/foxhound/generated";
import {
  createProtocolAdapter,
  failure,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import type { FoxhoundConsoleOperations } from "#generated/operations.ts";
import { FOXHOUND_RUN_VIEWS } from "#runs/run-views.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

const RUN_ID = "crawl-run";

function clientOf(adapter: ProtocolAdapter<FoxhoundConsoleOperations>): ConsoleClient {
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "foxhound",
  });
}

function viewOf(id: string): RunView {
  const view = FOXHOUND_RUN_VIEWS.find((candidate) => candidate.id === id);
  if (view === undefined) {
    throw new Error(`Foxhound's build has no view ${id}`);
  }
  return view;
}

function outcomeOf(
  position: number,
  url: string,
  kind: "succeeded" | "failed",
): CrawlRequestOutcome {
  const request = { depth: 0, id: `r-${String(position)}`, label: "home", url };
  return kind === "succeeded"
    ? {
        kind,
        position,
        attempts: 1,
        ended_at: "2026-10-24T09:00:00Z",
        errors: [],
        request,
        records: [],
      }
    : {
        kind,
        position,
        attempts: 3,
        ended_at: "2026-10-24T09:00:00Z",
        errors: [],
        request,
        error: { attempt: 3, code: "navigation_failed", message: "the page did not load" },
      };
}

/** The text of each row of the table the view shows, its header aside. */
function rows(): string[][] {
  return page
    .getByRole("row")
    .elements()
    .slice(1)
    .map((row) => [...row.querySelectorAll("td")].map((cell) => cell.textContent));
}

async function show(view: RunView, client: ConsoleClient): Promise<void> {
  await render(<ConsoleI18n>{view.content({ client, runId: RUN_ID })}</ConsoleI18n>);
}

test(
  "the Seeds view lists each seed of the run, added or not",
  { tags: ["story:12-203"] },
  async () => {
    const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
    adapter.answer("run.seeds", (params) => {
      expect(params.run_id).toBe(RUN_ID);
      return {
        seeds: [
          { kind: "added", id: "r-1", url: "https://shop.test/" },
          { kind: "not_added", reason: "duplicate", url: "https://SHOP.test:443/" },
        ],
      };
    });
    const client = clientOf(adapter);

    await show(viewOf("seeds"), client);

    await expect.element(page.getByRole("table", { name: "2 seeds" })).toBeVisible();
    expect(rows()).toEqual([
      ["https://shop.test/", "Added"],
      ["https://SHOP.test:443/", "Not added"],
    ]);
    client.close();
  },
);

test(
  "the Outcomes and Failed requests views list each request with its Label and outcome",
  { tags: ["story:12-203"] },
  async () => {
    const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
    adapter.answer("run.outcomes", {
      ended: true,
      next: 2,
      outcomes: [
        outcomeOf(1, "https://shop.test/", "succeeded"),
        outcomeOf(2, "https://shop.test/broken", "failed"),
      ],
    });
    adapter.answer("run.failed_requests", {
      requests: [outcomeOf(2, "https://shop.test/broken", "failed")],
    });
    const client = clientOf(adapter);

    await show(viewOf("outcomes"), client);
    await expect.element(page.getByText("2 outcomes The crawl ended.")).toBeVisible();
    await expect.element(page.getByRole("table", { name: "Request log" })).toBeVisible();
    expect(
      page
        .getByRole("rowheader")
        .elements()
        .map((cell) => cell.textContent),
    ).toEqual(["https://shop.test/", "https://shop.test/broken"]);

    await show(viewOf("failed-requests"), client);
    await expect.element(page.getByRole("table", { name: "1 failed request" })).toBeVisible();
    client.close();
  },
);

test("a view the engine refuses shows the refusal", { tags: ["story:12-203"] }, async () => {
  const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
  adapter.answer("run.seeds", failure("run_not_found", { run_id: RUN_ID }));
  const client = clientOf(adapter);

  await show(viewOf("seeds"), client);

  await expect.element(page.getByText("run_not_found", { exact: false })).toBeVisible();
  client.close();
});
