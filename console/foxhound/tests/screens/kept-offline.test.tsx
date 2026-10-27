// A Dataset view kept offline (spec #593, story 215; ADR 0031), against the
// in-memory adapter of Foxhound's engine and an offline cache kept in
// memory: "Keep offline" keeps the view's Records as the grid shows them,
// each its number and its shown fields alone, never its URL, its crawl
// request or its Provenance; a view opened with no Connection, as a page
// whose Daemon is stopped, shows what the cache keeps of the run's views,
// saying it is this browser's copy; and a page with no cache, under
// `console.offline_cache` off, offers no Keep offline.
import { createConsoleClient, type ConsoleClient, type Transport } from "@goodfetch/console/client";
import type { RunView } from "@goodfetch/console/console-build";
import { ConsoleI18n } from "@goodfetch/console/i18n";
import type { OfflineCache, OfflineKind } from "@goodfetch/console/offline";
import type { DatasetRecord, DatasetSummary } from "@goodfetch/foxhound/generated";
import { createProtocolAdapter } from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import { keptDatasetOf } from "#datasets/kept.ts";
import type { FoxhoundConsoleOperations } from "#generated/operations.ts";
import { FOXHOUND_RUN_VIEWS } from "#runs/run-views.tsx";

const RUN_ID = "crawl-run";

const BOOKS: DatasetSummary = {
  name: "books",
  schema: {
    type: "object",
    properties: { title: { type: "string" }, isbn: { type: "string" } },
  },
  key: ["isbn"],
  committed: 2,
  dropped: 0,
  rejected: 1,
};

/** The Record numbered `number` of `books`, its URL, crawl request and Provenance marked to be found. */
function record(number: number, value: Record<string, unknown>): DatasetRecord {
  return {
    number,
    value,
    request_id: `request-not-kept-${String(number)}`,
    url: `https://shop.test/${String(number)}?token=url-not-kept`,
    committed_at: "2026-10-24T09:00:00Z",
    provenance: { request_id: `provenance-not-kept-${String(number)}` },
  };
}

const DUNE = record(1, { title: "Dune", isbn: "1" });
const ALCHEMIST = record(2, { title: "الخيميائي", isbn: "2" });

/** An offline cache kept in memory, each row with the Dataset it was kept for. */
class MemoryCache implements OfflineCache {
  readonly rows = new Map<string, { value: unknown; dataset: string | undefined }>();

  async put(kind: OfflineKind, id: string, value: unknown, dataset?: string): Promise<void> {
    this.rows.set(`${kind}/${id}`, { value: structuredClone(value), dataset });
    return Promise.resolve();
  }

  async get(kind: OfflineKind, id: string): Promise<unknown> {
    return Promise.resolve(this.rows.get(`${kind}/${id}`)?.value);
  }

  async all(kind: OfflineKind): Promise<unknown[]> {
    return Promise.resolve(
      [...this.rows].filter(([name]) => name.startsWith(`${kind}/`)).map(([, row]) => row.value),
    );
  }

  async forgetDataset(dataset: string): Promise<void> {
    for (const [name, row] of this.rows) {
      if (row.dataset === dataset) {
        this.rows.delete(name);
      }
    }
    return Promise.resolve();
  }
}

/** A transport whose every Connection closes before the engine says anything, as a stopped Daemon's. */
const unreachable: Transport = {
  open(listener) {
    queueMicrotask(() => listener.close());
    return { send: () => {}, close: () => {} };
  },
};

function datasetsView(): RunView {
  const view = FOXHOUND_RUN_VIEWS.find((candidate) => candidate.id === "datasets");
  if (view === undefined) {
    throw new Error("Foxhound's build has no Datasets view");
  }
  return view;
}

/** A client over an engine answering `books` and its two Records. */
function answeringClient(): ConsoleClient {
  const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
  adapter.answer("dataset.list", { datasets: [BOOKS] });
  adapter.answer("dataset.page", { records: [DUNE, ALCHEMIST], through: 2 });
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: { visible: true, watch: () => () => {} },
    product: "foxhound",
  });
}

/** A client whose Daemon is stopped. */
function unreachableClient(): ConsoleClient {
  return createConsoleClient({
    transport: unreachable,
    bodyFetcher: async () => Promise.reject(new Error("no bodies offline")),
    visibility: { visible: true, watch: () => () => {} },
    product: "foxhound",
  });
}

/** The text of each cell of the Records grid's rows, its header aside. */
function gridRows(): string[][] {
  return page
    .getByRole("table", { name: "Records of books" })
    .getByRole("row")
    .elements()
    .slice(1)
    .map((row) => [...row.querySelectorAll("th, td")].map((cell) => cell.textContent));
}

test(
  "Keep offline keeps the view's Records as the grid shows them, never a field it does not show",
  { tags: ["story:12-215"] },
  async () => {
    const client = answeringClient();
    const cache = new MemoryCache();
    await render(
      <ConsoleI18n>
        {datasetsView().content({ client, runId: RUN_ID, offline: cache })}
      </ConsoleI18n>,
    );
    await expect.poll(gridRows).toEqual([
      ["1", "Dune", "1"],
      ["2", "الخيميائي", "2"],
    ]);
    await userEvent.click(page.getByRole("checkbox", { name: "isbn" }));
    await userEvent.click(page.getByRole("button", { name: "Apply" }));
    await expect.poll(gridRows).toEqual([
      ["1", "Dune"],
      ["2", "الخيميائي"],
    ]);

    await userEvent.click(page.getByRole("button", { name: "Keep offline" }));

    await expect
      .element(
        page.getByText("Kept offline in this browser: 2 Records, readable until you sign out"),
      )
      .toBeVisible();
    expect(await cache.all("dataset_view")).toEqual([
      {
        run_id: RUN_ID,
        dataset: BOOKS,
        columns: ["title"],
        records: [
          { number: 1, value: { title: "Dune" } },
          { number: 2, value: { title: "الخيميائي" } },
        ],
      },
    ]);
    expect(JSON.stringify([...cache.rows])).not.toMatch(/not-kept/);

    // An erasure of the Dataset forgets it by the name the view kept it under.
    await cache.forgetDataset(keptDatasetOf(RUN_ID, "books"));
    expect(await cache.all("dataset_view")).toEqual([]);
    client.close();
  },
);

test(
  "with no Connection, the Datasets view shows the views the browser keeps offline for the run",
  { tags: ["story:12-215"] },
  async () => {
    const cache = new MemoryCache();
    await cache.put("dataset_view", "other", {
      run_id: "another-run",
      dataset: { ...BOOKS, name: "elsewhere" },
      columns: ["title"],
      records: [{ number: 1, value: { title: "Not this run's" } }],
    });
    await cache.put("dataset_view", "kept", {
      run_id: RUN_ID,
      dataset: BOOKS,
      columns: ["title"],
      records: [
        { number: 1, value: { title: "Dune" } },
        { number: 2, value: { title: "الخيميائي" } },
      ],
    });
    const client = unreachableClient();

    await render(
      <ConsoleI18n>
        {datasetsView().content({ client, runId: RUN_ID, offline: cache })}
      </ConsoleI18n>,
    );

    await expect
      .element(page.getByText("This browser's offline copy", { exact: false }))
      .toBeVisible();
    await expect.poll(gridRows).toEqual([
      ["1", "Dune"],
      ["2", "الخيميائي"],
    ]);
    expect(page.getByText("elsewhere").elements()).toEqual([]);
    expect(page.getByRole("button", { name: "Keep offline" }).elements()).toEqual([]);
    client.close();
  },
);

test(
  "a page with no offline cache offers no Keep offline",
  { tags: ["story:12-215"] },
  async () => {
    const client = answeringClient();
    await render(<ConsoleI18n>{datasetsView().content({ client, runId: RUN_ID })}</ConsoleI18n>);
    await expect.poll(gridRows).toHaveLength(2);
    expect(page.getByRole("button", { name: "Keep offline" }).elements()).toEqual([]);
    client.close();
  },
);
