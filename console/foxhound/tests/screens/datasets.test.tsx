// Foxhound's Datasets view of a crawl run (spec #593, stories 204, 205,
// 207 and 233; ADRs 0031, 0071, 0073 and 0103), against the in-memory
// adapter of Foxhound's engine: it lists the crawl's Datasets with their
// schema, key and counts, and shows a Dataset's Records in a grid whose
// filters, sort and columns the engine applies, one `dataset.page` per
// change, counted here; each cell is isolated and kept out of
// translation; its render budget is the number of React commits a page
// read and a sort take, snapshotted under `<Profiler>`.
import { createConsoleClient, type ConsoleClient } from "@goodfetch/console/client";
import type { RunView } from "@goodfetch/console/console-build";
import { ConsoleI18n } from "@goodfetch/console/i18n";
import type {
  DatasetPage,
  DatasetPageParams,
  DatasetRecord,
  DatasetSummary,
} from "@goodfetch/foxhound/generated";
import {
  createProtocolAdapter,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { Profiler } from "react";
import { expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import type { FoxhoundConsoleOperations } from "#generated/operations.ts";
import { FOXHOUND_RUN_VIEWS } from "#runs/run-views.tsx";

const RUN_ID = "crawl-run";

const BOOKS: DatasetSummary = {
  name: "books",
  schema: {
    type: "object",
    required: ["title", "isbn"],
    properties: { title: { type: "string" }, isbn: { type: "string" } },
  },
  key: ["isbn"],
  committed: 3,
  dropped: 1,
  rejected: 2,
};
const NOTES: DatasetSummary = { name: "notes", key: [], committed: 0, dropped: 0, rejected: 0 };

/** The Record numbered `number` of `books`, holding `value`. */
function record(number: number, value: Record<string, unknown>): DatasetRecord {
  return {
    number,
    value,
    request_id: String(number),
    url: `https://shop.test/${String(number)}`,
    committed_at: "2026-10-24T09:00:00Z",
    provenance: { request_id: String(number) },
  };
}

const DUNE = record(1, { title: "Dune", isbn: "1" });
const ALCHEMIST = record(3, { title: "الخيميائي", isbn: "3" });

function datasetsView(): RunView {
  const view = FOXHOUND_RUN_VIEWS.find((candidate) => candidate.id === "datasets");
  if (view === undefined) {
    throw new Error("Foxhound's build has no Datasets view");
  }
  return view;
}

/** An adapter answering `dataset.list` with `books` and `notes`, and each `dataset.page` with `answer`. */
function adapterOf(
  answer: (params: DatasetPageParams) => DatasetPage,
): ProtocolAdapter<FoxhoundConsoleOperations> {
  const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
  adapter.answer("dataset.list", { datasets: [BOOKS, NOTES] });
  adapter.answer("dataset.page", answer);
  return adapter;
}

function clientOf(adapter: ProtocolAdapter<FoxhoundConsoleOperations>): ConsoleClient {
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: { visible: true, watch: () => () => {} },
    product: "foxhound",
  });
}

/** The params of each `dataset.page` the view sent. */
function pageReads(adapter: ProtocolAdapter<FoxhoundConsoleOperations>): unknown[] {
  return adapter.requests
    .filter((request) => request.method === "dataset.page")
    .map((request) => request.params);
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
  "the Datasets view lists each Dataset with its schema, its key and its committed, dropped and rejected counts",
  { tags: ["story:12-204"] },
  async () => {
    const adapter = adapterOf(() => ({ records: [DUNE], through: 3 }));
    const client = clientOf(adapter);

    await render(<ConsoleI18n>{datasetsView().content({ client, runId: RUN_ID })}</ConsoleI18n>);

    const list = page.getByRole("table", { name: "Datasets" });
    await expect.element(list).toBeVisible();
    const rows = list
      .getByRole("row")
      .elements()
      .slice(1)
      .map((row) => [...row.querySelectorAll("th, td")].map((cell) => cell.textContent));
    expect(rows).toEqual([
      ["books", JSON.stringify(BOOKS.schema), "isbn", "3", "1", "2"],
      ["notes", "Any JSON object", "None", "0", "0", "0"],
    ]);
    client.close();
  },
);

test(
  "the Records grid asks the engine to filter, sort and keep columns, one page read per change, and shows what it answers",
  { tags: ["story:12-205"] },
  async () => {
    // The engine's answer is shown as it is: the view filters and sorts nothing itself.
    const adapter = adapterOf((params) => ({
      records: params.sort === undefined ? [DUNE, ALCHEMIST] : [ALCHEMIST, DUNE],
      through: 3,
    }));
    const client = clientOf(adapter);
    await render(<ConsoleI18n>{datasetsView().content({ client, runId: RUN_ID })}</ConsoleI18n>);
    await expect.poll(gridRows).toEqual([
      ["1", "Dune", "1"],
      ["3", "الخيميائي", "3"],
    ]);
    expect(pageReads(adapter)).toEqual([{ run_id: RUN_ID, dataset: "books", limit: 500 }]);

    await userEvent.selectOptions(page.getByRole("combobox", { name: "Sort by" }), "title");
    await userEvent.selectOptions(page.getByRole("combobox", { name: "Order" }), "descending");
    await userEvent.selectOptions(page.getByRole("combobox", { name: "Filter on" }), "title");
    await userEvent.fill(page.getByRole("textbox", { name: "Contains" }), "u");
    await userEvent.selectOptions(page.getByRole("combobox", { name: "Wall" }), "without");
    await userEvent.click(page.getByRole("checkbox", { name: "isbn" }));
    await userEvent.click(page.getByRole("button", { name: "Apply" }));

    await expect.poll(() => pageReads(adapter).length).toBe(2);
    expect(pageReads(adapter)[1]).toEqual({
      run_id: RUN_ID,
      dataset: "books",
      limit: 500,
      sort: { field: "title", direction: "descending" },
      filter: { fields: [{ field: "title", op: "contains", text: "u" }], wall: "without" },
      columns: ["title"],
    });
    await expect.poll(gridRows).toEqual([
      ["3", "الخيميائي"],
      ["1", "Dune"],
    ]);
    client.close();
  },
);

test(
  "each cell of the Records grid is isolated with dir=auto and kept out of translation",
  { tags: ["story:12-207"] },
  async () => {
    const adapter = adapterOf(() => ({ records: [ALCHEMIST], through: 3 }));
    const client = clientOf(adapter);
    await render(<ConsoleI18n>{datasetsView().content({ client, runId: RUN_ID })}</ConsoleI18n>);
    await expect.poll(gridRows).toEqual([["3", "الخيميائي", "3"]]);

    const cells = page
      .getByRole("table", { name: "Records of books" })
      .getByRole("cell")
      .elements();
    expect(cells.map((cell) => [cell.getAttribute("dir"), cell.getAttribute("translate")])).toEqual(
      [
        ["auto", "no"],
        ["auto", "no"],
      ],
    );
    client.close();
  },
);

/** Two frames, so a commit the grid's measuring makes after its rows show still counts where it belongs. */
async function twoFrames(): Promise<void> {
  await new Promise((frame) => requestAnimationFrame(() => requestAnimationFrame(frame)));
}

/** The number of the grid's first row. */
function firstRow(): string | undefined {
  return gridRows()[0]?.[0];
}

test(
  "the Records grid's render budget: commits for a page read and for a sort",
  { tags: ["story:12-233"] },
  async () => {
    const adapter = adapterOf((params) => ({
      records: params.sort === undefined ? [DUNE, ALCHEMIST] : [ALCHEMIST, DUNE],
      through: 3,
    }));
    const client = clientOf(adapter);
    let commits = 0;

    await render(
      <ConsoleI18n>
        <Profiler id="datasets" onRender={() => (commits += 1)}>
          {datasetsView().content({ client, runId: RUN_ID })}
        </Profiler>
      </ConsoleI18n>,
    );
    await expect.poll(firstRow).toBe("1");
    await twoFrames();
    const pageRead = commits;

    commits = 0;
    await userEvent.selectOptions(page.getByRole("combobox", { name: "Sort by" }), "title");
    await userEvent.click(page.getByRole("button", { name: "Apply" }));
    await expect.poll(firstRow).toBe("3");
    await twoFrames();
    const sort = commits;

    expect({ pageRead, sort }).toMatchSnapshot();
    client.close();
  },
);
