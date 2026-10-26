// The Runs screen against the in-memory adapter of the protocol (spec
// #592, seam 1; ADRs 0031, 0036, 0071 and 0117): it lists every row
// `run.list` answers, a missing and an erased row as such, sends its filter
// and sort to the engine and keeps them in the page's address, and follows
// a live run's axes and status line from the event stream without reading
// the list again, reading it again when a run it does not list changes or
// a resync replaces its state. Its render budget is the number of React
// commits each action takes, snapshotted under `<Profiler>`.
import type { RunIndexRow, RunListParams, RunProgress } from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { Profiler } from "react";
import { expect, onTestFinished, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import { createConsoleClient, type ConsoleClient, type PageVisibility } from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import type { Address } from "#address.ts";
import { Shell } from "#shell.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

/** The page's address, which the test reads. */
class TestAddress implements Address {
  search: string;

  constructor(search: string) {
    this.search = search;
  }

  replace(search: string): void {
    this.search = search;
  }
}

function row(run_id: string, overrides: Partial<RunIndexRow> = {}): RunIndexRow {
  return {
    run_id,
    location: `runs/${run_id}`,
    kind: "collie",
    needs: "collie",
    tags: [],
    lifecycle: "running",
    started_at: "2026-10-01T08:00:00Z",
    presence: "present",
    replayed: false,
    ...overrides,
  };
}

const LIVE = row("live-run", { tags: ["team:shop"], progress_message: "Read 1 of 3 pages" });
const FINISHED = row("finished-run", {
  lifecycle: "finished",
  verdict: "verified",
  completeness: "complete",
  started_at: "2026-10-02T08:00:00Z",
  ended_at: "2026-10-02T09:00:00Z",
  tags: ["goodfetch.recipe:shop-prices"],
});
const MISSING = row("missing-run", {
  lifecycle: "stopped",
  verdict: "needs_review",
  completeness: "incomplete",
  ended_at: "2026-10-03T09:00:00Z",
  presence: "missing",
});
const ERASED = row("erased-run", {
  lifecycle: "failed",
  verdict: "rejected",
  completeness: "incomplete",
  ended_at: "2026-10-04T09:00:00Z",
  presence: "erased",
});

/** The params of each `run.list` the screen sent. */
function listed(adapter: ProtocolAdapter): unknown[] {
  return adapter.requests
    .filter((request) => request.method === "run.list")
    .map((request) => request.params);
}

/** A client of `adapter`'s engine, its "Needs you" list empty. */
function clientOf(adapter: ProtocolAdapter): ConsoleClient {
  adapter.answer("needs_you.list", { items: [] });
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "collie",
  });
}

/** The row header of the run `runId`, once the table lists it. */
async function headerOf(runId: string): Promise<Element> {
  await expect.poll(listedIds).toContain(runId);
  const header = page
    .getByRole("rowheader")
    .elements()
    .find((cell) => cell.querySelector("code")?.textContent === runId);
  if (header === undefined) {
    throw new Error(`invariant: the table lists ${runId}`);
  }
  return header;
}

/** The text of each cell of the row of `runId`. */
async function cells(runId: string): Promise<string[]> {
  const found = (await headerOf(runId)).closest("tr");
  return [...(found?.querySelectorAll("td") ?? [])].map((cell) => cell.textContent);
}

/** The Run IDs the table lists, in its order. */
function listedIds(): string[] {
  return page
    .getByRole("rowheader")
    .elements()
    .map((cell) => cell.querySelector("code")?.textContent ?? "");
}

async function renderRuns(client: ConsoleClient, address: Address): Promise<void> {
  await render(
    <ConsoleI18n>
      <Shell client={client} route="/runs" address={address} />
    </ConsoleI18n>,
  );
}

test(
  "the row of a run that replayed a Response recording carries the replayed mark, and a live run's row none",
  { tags: ["story:12-15", "screen:/runs"] },
  async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("run.list", {
      runs: [row("replayed-run", { lifecycle: "finished", replayed: true }), FINISHED],
    });
    const client = clientOf(adapter);

    await renderRuns(client, new TestAddress(""));

    expect((await headerOf("replayed-run")).textContent).toContain("Replayed");
    expect((await headerOf("finished-run")).textContent).not.toContain("Replayed");
    client.close();
  },
);

test(
  "each row names the Product its run needs, and a live run's row the engine that owns it",
  { tags: ["story:12-120", "screen:/runs"] },
  async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("run.list", {
      runs: [
        row("crawl-run", {
          needs: "foxhound",
          kind: "crawl",
          owner: { daemon: { tcp: "127.0.0.1:7801" } },
        }),
        FINISHED,
      ],
    });
    const client = clientOf(adapter);

    await renderRuns(client, new TestAddress(""));

    expect(await cells("crawl-run")).toEqual(
      expect.arrayContaining(["Foxhound", "the Daemon at 127.0.0.1:7801"]),
    );
    expect(await cells("finished-run")).toEqual(expect.arrayContaining(["Collie", ""]));
    client.close();
  },
);

test(
  "the Runs screen lists every row of the Run index with its three axes, a missing and an erased row as such",
  { tags: ["story:12-10", "story:12-13", "screen:/runs"] },
  async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("run.list", { runs: [ERASED, MISSING, FINISHED, LIVE] });
    const client = clientOf(adapter);

    await renderRuns(client, new TestAddress(""));

    await expect.poll(listedIds).toEqual(["erased-run", "missing-run", "finished-run", "live-run"]);
    expect(await cells("live-run")).toEqual(
      expect.arrayContaining(["running", "Read 1 of 3 pages", "Collie"]),
    );
    expect(await cells("finished-run")).toEqual(
      expect.arrayContaining(["finished", "verified", "complete", "shop-prices"]),
    );
    expect((await headerOf("live-run")).querySelector("a")?.getAttribute("href")).toBe(
      "/runs/live-run",
    );
    expect((await headerOf("missing-run")).textContent).toContain("Run file missing");
    expect((await headerOf("erased-run")).textContent).toContain("Erased");
    expect(listed(adapter)).toEqual([{}]);
    client.close();
  },
);

test(
  "the filter in the address is sent to run.list, and a new one replaces it in the address",
  { tags: ["story:12-12", "screen:/runs"] },
  async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("run.list", (params: RunListParams) => ({
      runs: params.text === undefined ? [ERASED, MISSING] : [MISSING],
    }));
    const client = clientOf(adapter);
    const address = new TestAddress("?completeness=incomplete");

    await renderRuns(client, address);

    await expect.poll(listedIds).toEqual(["erased-run", "missing-run"]);
    await expect
      .element(page.getByRole("combobox", { name: "Completeness" }))
      .toHaveValue("incomplete");
    await userEvent.fill(page.getByRole("searchbox", { name: "Search" }), "missing");
    await userEvent.selectOptions(page.getByRole("combobox", { name: "Sort by" }), "ended_at");
    await userEvent.click(page.getByRole("button", { name: "Apply" }));

    await expect.poll(listedIds).toEqual(["missing-run"]);
    expect(listed(adapter)).toEqual([
      { completeness: "incomplete" },
      { completeness: "incomplete", text: "missing", sort: "ended_at" },
    ]);
    expect(address.search).toBe("?completeness=incomplete&text=missing&sort=ended_at");
    client.close();
  },
);

/** The progress the engine reports for `runId`, its status line `message`. */
function progress(runId: string, message: string, position: number): RunProgress {
  return {
    run_id: runId,
    position,
    at: "2026-10-01T08:05:00Z",
    lifecycle: "running",
    writer: "live",
    epoch: 1,
    pauses: [],
    message,
    last: false,
  };
}

test(
  "a live run's row changes as an event batch arrives, without reading the list again",
  { tags: ["story:12-14", "screen:/runs"] },
  async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("run.list", { runs: [FINISHED, LIVE] });
    const client = clientOf(adapter);
    await renderRuns(client, new TestAddress(""));
    await expect.poll(listedIds).toEqual(["finished-run", "live-run"]);
    await expect
      .poll(() => adapter.requests.some((request) => request.method === "events.subscribe"))
      .toBe(true);

    adapter.push({ event: "run.progress", data: progress("live-run", "Read 3 of 3 pages", 1) }, 1);
    adapter.push(
      {
        event: "run.axes_changed",
        data: {
          run_id: "live-run",
          lifecycle: "finished",
          verdict: "verified",
          completeness: "complete",
        },
      },
      2,
    );

    await expect
      .poll(() => cells("live-run"))
      .toEqual(expect.arrayContaining(["finished", "verified", "complete", "Read 3 of 3 pages"]));
    expect(listed(adapter)).toHaveLength(1);
    client.close();
  },
);

test(
  "a run the screen does not list, or a resync, makes it read the list again",
  { tags: ["story:12-14", "story:12-123", "screen:/runs"] },
  async () => {
    const adapter = createProtocolAdapter();
    const index = [LIVE];
    adapter.answer("run.list", () => ({ runs: [...index] }));
    const client = clientOf(adapter);
    await renderRuns(client, new TestAddress(""));
    await expect.poll(listedIds).toEqual(["live-run"]);
    await expect
      .poll(() => adapter.requests.some((request) => request.method === "events.subscribe"))
      .toBe(true);

    index.unshift(row("new-run", { started_at: "2026-10-05T08:00:00Z" }));
    adapter.push(
      { event: "run.axes_changed", data: { run_id: "new-run", lifecycle: "running" } },
      1,
    );

    await expect.poll(listedIds).toEqual(["new-run", "live-run"]);

    index.splice(0, index.length, FINISHED);
    adapter.restart();

    await expect.poll(listedIds).toEqual(["finished-run"]);
    client.close();
  },
);

test(
  "the Runs screen's render budget: commits per event batch, filtering and sorting",
  { tags: ["screen:/runs"] },
  async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("run.list", (params: RunListParams) => ({
      runs: params.lifecycle === "finished" ? [FINISHED] : [FINISHED, LIVE],
    }));
    const client = clientOf(adapter);
    let commits = 0;
    await render(
      <ConsoleI18n>
        <Profiler id="runs" onRender={() => (commits += 1)}>
          <Shell client={client} route="/runs" address={new TestAddress("")} />
        </Profiler>
      </ConsoleI18n>,
    );
    await expect.poll(listedIds).toEqual(["finished-run", "live-run"]);
    await expect
      .poll(() => adapter.requests.some((request) => request.method === "events.subscribe"))
      .toBe(true);
    /** The commits `action` takes until `settled` holds. */
    const committed = async (action: () => Promise<void>, settled: () => Promise<void>) => {
      commits = 0;
      await action();
      await settled();
      return commits;
    };

    const eventBatch = await committed(
      async () => {
        for (const [position, message] of ["Read 2 of 3 pages", "Read 3 of 3 pages"].entries()) {
          adapter.push(
            { event: "run.progress", data: progress("live-run", message, position + 1) },
            position + 1,
          );
        }
        adapter.push(
          { event: "run.axes_changed", data: { run_id: "live-run", lifecycle: "stopped" } },
          3,
        );
      },
      async () => {
        await expect
          .poll(() => cells("live-run"))
          .toEqual(expect.arrayContaining(["stopped", "Read 3 of 3 pages"]));
      },
    );
    const filtering = await committed(
      async () => {
        await userEvent.selectOptions(
          page.getByRole("combobox", { name: "Lifecycle" }),
          "finished",
        );
        await userEvent.click(page.getByRole("button", { name: "Apply" }));
      },
      async () => {
        await expect.poll(listedIds).toEqual(["finished-run"]);
      },
    );
    const sorting = await committed(
      async () => {
        await userEvent.selectOptions(page.getByRole("combobox", { name: "Sort by" }), "run_id");
        await userEvent.click(page.getByRole("button", { name: "Apply" }));
      },
      async () => {
        await expect.poll(() => listed(adapter).length).toBe(3);
        await expect.poll(listedIds).toEqual(["finished-run"]);
      },
    );

    expect({ eventBatch, filtering, sorting }).toMatchSnapshot();
    client.close();
  },
);

/** The rows of a Run index of `count` finished runs. */
function runIndex(count: number): RunIndexRow[] {
  return Array.from({ length: count }, (_, position) =>
    row(`run-${String(position).padStart(5, "0")}`, { lifecycle: "finished" }),
  );
}

test(
  "a 10,000-row Runs list renders only the rows in view, and the last ones once scrolled to",
  { tags: ["story:12-136", "screen:/runs"] },
  async () => {
    // The next test's page starts at its top, as a page opens: a scroll
    // left over would commit in its render budget.
    onTestFinished(() => window.scrollTo({ top: 0 }));
    const adapter = createProtocolAdapter();
    adapter.answer("run.list", { runs: runIndex(10_000) });
    const client = clientOf(adapter);

    await renderRuns(client, new TestAddress(""));

    await expect.poll(listedIds).toContain("run-00000");
    const rendered = listedIds().length;
    expect(rendered).toBeLessThan(100);
    expect(listedIds()).not.toContain("run-09999");
    expect(page.getByRole("table", { name: "Runs" }).element().getAttribute("aria-rowcount")).toBe(
      "10001",
    );

    window.scrollTo({ top: document.documentElement.scrollHeight });

    await expect.poll(listedIds).toContain("run-09999");
    expect(listedIds().length).toBeLessThan(100);
    expect({ rendered }).toMatchSnapshot();
    client.close();
  },
);
