// The SQL tab (spec #593, stories 210 and 211), against the in-memory
// adapter of Foxhound's engine: it runs the statement over the runs the
// person selects, this run first, and shows the result's columns and one
// page of rows, the rest counted; a refusal shows as the Console shows any.
// Its render budget (ADR 0071) is the commits a result of
// `dataset.query.max_rows` rows takes and the rows it puts in the DOM,
// snapshotted under `<Profiler>`.
import type { ConsoleClient, PageVisibility } from "@goodfetch/console/client";
import { createConsoleClient } from "@goodfetch/console/client";
import { ConsoleI18n } from "@goodfetch/console/i18n";
import {
  createProtocolAdapter,
  failure,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { Profiler } from "react";
import { expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import type { FoxhoundConsoleOperations } from "#generated/operations.ts";
import { FOXHOUND_RUN_VIEWS } from "#runs/run-views.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

const RUN_ID = "shop-tuesday";

function clientOf(adapter: ProtocolAdapter<FoxhoundConsoleOperations>): ConsoleClient {
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "foxhound",
  });
}

function sqlTab(client: ConsoleClient) {
  const view = FOXHOUND_RUN_VIEWS.find((candidate) => candidate.id === "sql");
  if (view === undefined) {
    throw new Error("Foxhound's build has no SQL tab");
  }
  return view.content({ client, runId: RUN_ID });
}

/** The text of each row of the result, its header aside. */
function rows(): string[][] {
  return page
    .getByRole("row")
    .elements()
    .slice(1)
    .map((row) => [...row.querySelectorAll("td")].map((cell) => cell.textContent));
}

async function runStatement(sql: string, runs: string): Promise<void> {
  await userEvent.fill(page.getByLabelText("Statement"), sql);
  await userEvent.fill(page.getByLabelText("Runs it reads"), runs);
  await userEvent.click(page.getByRole("button", { name: "Run the statement" }));
}

test(
  "the SQL tab runs a statement over the runs the person selects and shows its columns and rows",
  { tags: ["story:12-210"] },
  async () => {
    const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
    const asked: { sql: string; runs: readonly string[] }[] = [];
    adapter.answer("dataset.query", (params) => {
      asked.push({ sql: params.sql, runs: params.runs });
      return {
        columns: [
          { name: "title", type: "VARCHAR" },
          { name: "runs", type: "BIGINT" },
        ],
        rows: [
          ["Café 1", 2],
          ["كتاب ٢", 1],
        ],
        total: 2,
      };
    });
    const client = clientOf(adapter);
    await render(<ConsoleI18n>{sqlTab(client)}</ConsoleI18n>);

    await runStatement(
      "SELECT title, count(*) FROM records('shop-monday') GROUP BY title",
      `${RUN_ID} shop-monday, ${RUN_ID}`,
    );

    await expect.element(page.getByText("Rows 1 to 2 of 2.")).toBeVisible();
    expect(asked).toEqual([
      {
        sql: "SELECT title, count(*) FROM records('shop-monday') GROUP BY title",
        runs: [RUN_ID, "shop-monday"],
      },
    ]);
    expect(
      page
        .getByRole("columnheader")
        .elements()
        .map((cell) => cell.textContent),
    ).toEqual(["title VARCHAR", "runs BIGINT"]);
    expect(rows()).toEqual([
      ["Café 1", "2"],
      ["كتاب ٢", "1"],
    ]);
    for (const cell of page.getByRole("cell").elements()) {
      expect([cell.getAttribute("dir"), cell.getAttribute("translate")]).toEqual(["auto", "no"]);
    }
    client.close();
  },
);

test("a statement the engine refuses shows its refusal", { tags: ["story:12-211"] }, async () => {
  const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
  adapter.answer("dataset.query", failure("query_invalid", { reason: "not_a_read" }));
  const client = clientOf(adapter);
  await render(<ConsoleI18n>{sqlTab(client)}</ConsoleI18n>);

  await runStatement("INSERT INTO kept VALUES (1)", RUN_ID);

  await expect.element(page.getByText("query_invalid", { exact: false })).toBeVisible();
  client.close();
});

test(
  "a result of dataset.query.max_rows rows shows one page of them, the rest counted, within its render budget",
  { tags: ["story:12-211"] },
  async () => {
    const adapter = createProtocolAdapter<FoxhoundConsoleOperations>({ product: "foxhound" });
    adapter.answer("dataset.query", {
      columns: [
        { name: "number", type: "BIGINT" },
        { name: "title", type: "VARCHAR" },
      ],
      rows: Array.from({ length: 10_000 }, (_, index) => [index + 1, `Café ${String(index + 1)}`]),
      total: 250_000,
    });
    const client = clientOf(adapter);
    let commits = 0;
    await render(
      <ConsoleI18n>
        <Profiler id="sql-tab" onRender={() => (commits += 1)}>
          {sqlTab(client)}
        </Profiler>
      </ConsoleI18n>,
    );
    commits = 0;

    await runStatement("SELECT number, title FROM records('shop-tuesday')", RUN_ID);
    await expect
      .element(page.getByText("Rows 1 to 100 of 10000 returned; the query produced 250000."))
      .toBeVisible();
    await new Promise((frame) => requestAnimationFrame(() => requestAnimationFrame(frame)));
    const ran = { commits, rows: rows().length };
    commits = 0;
    await userEvent.click(page.getByRole("button", { name: "Next rows" }));
    await expect
      .element(page.getByText("Rows 101 to 200 of 10000", { exact: false }))
      .toBeVisible();
    await new Promise((frame) => requestAnimationFrame(() => requestAnimationFrame(frame)));
    const paged = { commits, rows: rows().length, first: rows()[0] };

    expect({ ran, paged }).toEqual({
      ran: { commits: 2, rows: 100 },
      paged: { commits: 1, rows: 100, first: ["101", "Café 101"] },
    });
    client.close();
  },
);
