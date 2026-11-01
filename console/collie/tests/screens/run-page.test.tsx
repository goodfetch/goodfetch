// A run's page against the in-memory adapter of the protocol (spec #592,
// seam 1; ADRs 0029, 0041, 0102, 0107 and 0122): it opens at the address a
// Run report's `consoleUrl` gives, shows the run's Lifecycle, Verdict and
// Completeness as three values, marks a run that can spend money with
// nothing capping it *uncapped*, shows its writer, run epoch, last
// Checkpoint, each Human pause waiting and its worst storage level, offers
// stop and checkpoint on a live run and the resume command on an
// interrupted one, and follows the run as `run.progress` reports it. Its
// render budget is the number of React commits opening it and an axis
// change take, snapshotted under `<Profiler>`.
import type {
  RunIndexRow,
  RunProgress,
  RunProgressParams,
  RunReport,
  RunUsage,
} from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  failure,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { Profiler } from "react";
import { expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import { createConsoleClient, type ConsoleClient, type PageVisibility } from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

const RUN_ID = "nightly";

function progressOf(overrides: Partial<RunProgress> = {}): RunProgress {
  return {
    run_id: RUN_ID,
    position: 1,
    at: "2026-10-24T09:00:00Z",
    lifecycle: "running",
    writer: "live",
    epoch: 1,
    pauses: [],
    last: false,
    ...overrides,
  };
}

function usageOf(overrides: Partial<RunUsage> = {}): RunUsage {
  return {
    browser_time: 0,
    model: { calls: 0, tokens: 0 },
    proxy_bytes: 0,
    requests: 0,
    ...overrides,
  };
}

function rowOf(overrides: Partial<RunIndexRow> = {}): RunIndexRow {
  return {
    run_id: RUN_ID,
    location: `runs/${RUN_ID}`,
    kind: "collie",
    needs: "collie",
    tags: [],
    lifecycle: "running",
    started_at: "2026-10-24T08:00:00Z",
    presence: "present",
    replayed: false,
    ...overrides,
  };
}

function reportOf(overrides: Partial<RunReport> = {}): RunReport {
  return {
    run_id: RUN_ID,
    lifecycle: "stopped",
    verdict: "needs_review",
    completeness: "incomplete",
    started_at: "2026-10-24T08:00:00Z",
    ended_at: "2026-10-24T09:10:00Z",
    usage: usageOf(),
    ...overrides,
  };
}

/** The values `run.progress` gives after the first, each sent once the page waits for it. */
class ProgressFeed {
  first: RunProgress;
  readonly #waiting: ((next: RunProgress) => void)[] = [];
  /** Each `send` waiting for the page to read. */
  readonly #senders: (() => void)[] = [];

  constructor(first: RunProgress) {
    this.first = first;
  }

  readonly answer = (params: RunProgressParams): RunProgress | Promise<RunProgress> =>
    params.after === undefined || params.after === null
      ? this.first
      : new Promise((resolve) => {
          this.#waiting.push(resolve);
          for (const sender of this.#senders.splice(0)) {
            sender();
          }
        });

  /** Answers the page's waiting read with `next`, once it waits. */
  async send(next: RunProgress): Promise<void> {
    while (this.#waiting.length === 0) {
      await new Promise<void>((resolve) => this.#senders.push(resolve));
    }
    this.first = next;
    this.#waiting.shift()?.(next);
  }
}

/** The engine of a run whose progress, Usage and Run index row are these. */
function engineOf({
  progress = progressOf(),
  usage = usageOf(),
  row = rowOf(),
}: {
  readonly progress?: RunProgress | ProgressFeed;
  readonly usage?: RunUsage;
  readonly row?: RunIndexRow | (() => RunIndexRow);
} = {}): ProtocolAdapter {
  const adapter = createProtocolAdapter();
  adapter.answer("needs_you.list", { items: [] });
  // A read after a position waits for a change, as the engine's does.
  const feed = progress instanceof ProgressFeed ? progress : new ProgressFeed(progress);
  adapter.answer("run.progress", feed.answer);
  adapter.answer("run.usage", usage);
  adapter.answer("run.list", () => ({ runs: [typeof row === "function" ? row() : row] }));
  // An ended run's page reads its Run report, which the report's own tests show.
  adapter.answer("run.report", () => {
    const listed = typeof row === "function" ? row() : row;
    return reportOf({
      lifecycle: listed.lifecycle,
      ...(listed.verdict === undefined || listed.verdict === null
        ? {}
        : { verdict: listed.verdict }),
      ...(listed.completeness === undefined || listed.completeness === null
        ? {}
        : { completeness: listed.completeness }),
      usage,
    });
  });
  return adapter;
}

function clientOf(adapter: ProtocolAdapter): ConsoleClient {
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "collie",
  });
}

/** Renders the shell at the page of `runId`, as a Run report's `consoleUrl` opens it. */
async function openRunPage(client: ConsoleClient, runId = RUN_ID): Promise<void> {
  await render(
    <ConsoleI18n>
      <Shell client={client} route={`/runs/${runId}`} />
    </ConsoleI18n>,
  );
}

/** Each term the page shows, with its value's text. */
function facts(): Record<string, string> {
  return Object.fromEntries(
    page
      .getByRole("term")
      .elements()
      .map((term) => [term.textContent, term.nextElementSibling?.textContent ?? ""]),
  );
}

/** The methods the page called, `needs_you.list` and the subscriptions aside. */
function called(adapter: ProtocolAdapter): string[] {
  return adapter.requests
    .map((request) => request.method)
    .filter((method) => method.startsWith("run."));
}

test.each([
  {
    name: "running, with no Verdict or Completeness yet",
    row: rowOf(),
    axes: ["running", "none yet", "none yet"],
  },
  {
    name: "finished but needs review",
    row: rowOf({ lifecycle: "finished", verdict: "needs_review", completeness: "complete" }),
    axes: ["finished", "needs review", "complete"],
  },
  {
    name: "stopped, verified but incomplete",
    row: rowOf({ lifecycle: "stopped", verdict: "verified", completeness: "incomplete" }),
    axes: ["stopped", "verified", "incomplete"],
  },
  {
    name: "finished, verified and complete",
    row: rowOf({ lifecycle: "finished", verdict: "verified", completeness: "complete" }),
    axes: ["finished", "verified", "complete"],
  },
  {
    name: "failed and rejected",
    row: rowOf({ lifecycle: "failed", verdict: "rejected", completeness: "unverified" }),
    axes: ["failed", "rejected", "unverified"],
  },
])(
  "a run $name shows its Lifecycle, Verdict and Completeness as three values",
  { tags: ["story:12-11", "screen:/runs/$run"] },
  async ({ row, axes }) => {
    const ended = row.lifecycle !== "running";
    const adapter = engineOf({
      row,
      progress: progressOf({
        lifecycle: row.lifecycle,
        writer: ended ? "none" : "live",
        last: ended,
      }),
    });
    const client = clientOf(adapter);

    await openRunPage(client);

    await expect
      .element(page.getByRole("heading", { level: 1 }))
      .toHaveTextContent(`Run ${RUN_ID}`);
    await expect.poll(facts).toMatchObject({
      Lifecycle: axes[0],
      Verdict: axes[1],
      Completeness: axes[2],
    });
    client.close();
  },
);

test(
  "the page of a Run ID the Run index does not list says so",
  { tags: ["story:12-16", "screen:/runs/$run"] },
  async () => {
    const adapter = engineOf();
    adapter.answer("run.progress", failure("run_not_found", { run_id: "no-such-run" }));
    const client = clientOf(adapter);

    await openRunPage(client, "no-such-run");

    await expect
      .element(page.getByRole("alert"))
      .toHaveTextContent("No run has the Run ID no-such-run: the Run index does not list it.");
    client.close();
  },
);

/** ADR 0107's Model allowance in `RunUsage`, which no Limit is. */
const withAllowance: RunUsage = usageOf({
  uncapped: true,
  model_allowance: [
    {
      kind: "price",
      currency: "USD",
      value: 2_000_000,
      remains: 500_000,
      source: { key: "ai.allowance.price", value: { amount: 2, currency: "USD" }, layer: "run" },
    },
  ],
});

test.each([
  {
    name: "can spend with no cap and no Model allowance",
    usage: usageOf({ uncapped: true }),
    marked: true,
  },
  { name: "can spend with no cap under a Model allowance", usage: withAllowance, marked: true },
  { name: "spends only under its Limits", usage: usageOf({ uncapped: false }), marked: false },
  { name: "has spent nothing", usage: usageOf(), marked: false },
])(
  "a run that $name is marked uncapped: $marked",
  { tags: ["story:12-18", "screen:/runs/$run"] },
  async ({ usage, marked }) => {
    const client = clientOf(engineOf({ usage }));

    await openRunPage(client);

    await expect.poll(facts).toHaveProperty("Lifecycle", "running");
    expect(page.getByText("Uncapped", { exact: true }).elements()).toHaveLength(marked ? 1 : 0);
    client.close();
  },
);

test(
  "a run's page shows its writer, run epoch, last Checkpoint, each Human pause waiting and its worst storage level",
  { tags: ["story:12-19", "screen:/runs/$run"] },
  async () => {
    const client = clientOf(
      engineOf({
        progress: progressOf({
          epoch: 3,
          checkpoint_at: "2026-10-24T08:45:00Z",
          message: "Read 12 of 40 pages",
          storage: { level: "low", folder: "/srv/goodfetch/runs/nightly", free_bytes: 52_428_800 },
          pauses: [
            {
              session: "shop",
              page: "page-1",
              reason: "Log in with the MFA code",
              raised_by: { by: "program" },
              since: "2026-10-24T08:50:00Z",
            },
            {
              session: "shop",
              page: "page-2",
              raised_by: { by: "engine", step: "challenge_ladder" },
              since: "2026-10-24T08:55:00Z",
              deadline: "2026-10-24T09:55:00Z",
            },
          ],
        }),
      }),
    );

    await openRunPage(client);

    await expect.poll(facts).toMatchObject({
      Writer: "live: an engine works this run",
      "Run epoch": "3",
      Status: "Read 12 of 40 pages",
    });
    const checkpoint = page
      .getByRole("term")
      .elements()
      .find((term) => term.textContent === "Last Checkpoint")?.nextElementSibling;
    expect(checkpoint?.querySelector("time")?.getAttribute("datetime")).toBe(
      "2026-10-24T08:45:00Z",
    );
    const storage = facts()["Storage"] ?? "";
    expect(storage).toContain("low");
    expect(storage).toContain("/srv/goodfetch/runs/nightly");
    await expect.element(page.getByText("/srv/goodfetch/runs/nightly")).toBeVisible();
    expect(
      document.querySelector("data[value='52428800']"),
      "the free space, in bytes, beside its formatted text",
    ).not.toBeNull();
    const pauses = page.getByRole("list", { name: "Human pauses waiting" }).getByRole("listitem");
    await expect
      .poll(() => pauses.elements().map((pause) => pause.textContent))
      .toEqual([
        expect.stringContaining("Log in with the MFA code"),
        expect.stringContaining("the Challenge ladder"),
      ]);
    client.close();
  },
);

test(
  "a live run's page stops it and writes a Checkpoint",
  { tags: ["story:12-20", "screen:/runs/$run"] },
  async () => {
    const adapter = engineOf();
    adapter.answer("run.checkpoint", {
      run_id: RUN_ID,
      digest: "sha256:0f",
      journal_position: 4,
      location: "/srv/goodfetch/runs/nightly/checkpoint.sqlite",
      run_epoch: 1,
      size: 4096,
      time: "2026-10-24T09:01:00Z",
    });
    adapter.answer("run.stop", reportOf());
    const client = clientOf(adapter);
    await openRunPage(client);
    await expect.poll(facts).toHaveProperty("Writer", "live: an engine works this run");

    await userEvent.click(page.getByRole("button", { name: "Write a Checkpoint" }));
    await expect.element(page.getByRole("status").filter({ hasText: "Checkpoint" })).toBeVisible();
    await userEvent.click(page.getByRole("button", { name: "Stop" }));

    await expect
      .poll(() => called(adapter))
      .toEqual(expect.arrayContaining(["run.checkpoint", "run.stop"]));
    expect(
      adapter.requests
        .filter((request) => request.method === "run.checkpoint" || request.method === "run.stop")
        .map((request) => request.params),
    ).toEqual([{ run_id: RUN_ID }, { run_id: RUN_ID }]);
    expect(page.getByRole("button", { name: "Copy resume command" }).elements()).toHaveLength(0);
    client.close();
  },
);

test(
  "an interrupted run's page offers no stop and copies its resume command",
  { tags: ["story:12-20", "screen:/runs/$run"] },
  async () => {
    const client = clientOf(
      engineOf({ progress: progressOf({ writer: "none", epoch: 2, last: true }) }),
    );

    await openRunPage(client);

    await expect.poll(facts).toHaveProperty("Writer", "none: no engine works this run");
    await expect
      .element(page.getByText(`goodfetch-collie run resume ${RUN_ID}`, { exact: true }))
      .toBeVisible();
    expect(page.getByRole("button", { name: "Stop" }).elements()).toHaveLength(0);
    // The component project's browser grants no clipboard permission, so the
    // copy is refused here and the page says how to copy it by hand; the
    // served test reads the clipboard the copy wrote.
    await userEvent.click(page.getByRole("button", { name: "Copy resume command" }));
    await expect
      .element(page.getByRole("status").filter({ hasText: "resume command" }))
      .toHaveTextContent("The browser refused to copy the resume command: select it and copy it.");
    client.close();
  },
);

test(
  "a run's page follows its progress, and reads its axes again once its Lifecycle changes",
  { tags: ["story:12-11", "story:12-14", "story:12-19", "screen:/runs/$run"] },
  async () => {
    const feed = new ProgressFeed(progressOf({ message: "Read 1 of 3 pages" }));
    let row = rowOf();
    const adapter = engineOf({ progress: feed, row: () => row });
    const client = clientOf(adapter);
    await openRunPage(client);
    await expect.poll(facts).toHaveProperty("Status", "Read 1 of 3 pages");

    await feed.send(progressOf({ position: 2, message: "Read 3 of 3 pages" }));
    await expect.poll(facts).toHaveProperty("Status", "Read 3 of 3 pages");
    row = rowOf({ lifecycle: "finished", verdict: "verified", completeness: "complete" });
    await feed.send(progressOf({ position: 3, lifecycle: "finished", writer: "none", last: true }));

    await expect.poll(facts).toMatchObject({
      Lifecycle: "finished",
      Verdict: "verified",
      Completeness: "complete",
      Writer: "none: no engine works this run",
    });
    expect(called(adapter).filter((method) => method === "run.list")).toHaveLength(2);
    client.close();
  },
);

test(
  "a run's page's render budget: commits to open it and for an axis change",
  { tags: ["screen:/runs/$run"] },
  async () => {
    const feed = new ProgressFeed(progressOf());
    let row = rowOf();
    const client = clientOf(engineOf({ progress: feed, row: () => row }));
    let commits = 0;
    await render(
      <ConsoleI18n>
        <Profiler id="run-page" onRender={() => (commits += 1)}>
          <Shell client={client} route={`/runs/${RUN_ID}`} />
        </Profiler>
      </ConsoleI18n>,
    );
    await expect.poll(facts).toHaveProperty("Lifecycle", "running");
    const opening = commits;

    commits = 0;
    row = rowOf({ lifecycle: "stopped", verdict: "needs_review", completeness: "incomplete" });
    await feed.send(progressOf({ position: 2, lifecycle: "stopped", writer: "none", last: true }));
    await expect.poll(facts).toHaveProperty("Verdict", "needs review");
    const axisChange = commits;

    expect({ opening, axisChange }).toMatchSnapshot();
    client.close();
  },
);

test(
  "a live run's page names the Product the run needs and the engine that owns it",
  { tags: ["story:12-120", "screen:/runs/$run"] },
  async () => {
    const owner = { daemon: { unix: "/home/ada/.local/share/goodfetch/0.1/daemon.sock" } };
    const client = clientOf(engineOf({ row: rowOf({ needs: "foxhound", owner }) }));

    await openRunPage(client);

    await expect.poll(facts).toMatchObject({
      "Product it needs": "Foxhound",
      "Owning engine": "the Daemon at /home/ada/.local/share/goodfetch/0.1/daemon.sock",
    });
    client.close();
  },
);

test(
  "on Collie's engine, a run that needs Foxhound keeps its common view, and each Foxhound view shows not_in_product with its fix",
  { tags: ["story:12-121", "screen:/runs/$run"] },
  async () => {
    const adapter = engineOf({
      row: rowOf({
        needs: "foxhound",
        kind: "crawl",
        lifecycle: "finished",
        verdict: "verified",
        completeness: "incomplete",
      }),
      progress: progressOf({ lifecycle: "finished", writer: "none", last: true }),
    });
    const client = clientOf(adapter);

    await openRunPage(client);

    const tabs = page.getByRole("tablist", { name: "Run views" }).getByRole("tab");
    await expect
      .poll(() => tabs.elements().map((tab) => tab.textContent))
      .toEqual(["Run", "Records", "Outcomes", "Failed requests", "Seeds", "Rejected records"]);
    await expect.poll(facts).toHaveProperty("Lifecycle", "finished");

    await userEvent.click(page.getByRole("tab", { name: "Records" }));

    const panel = page.getByRole("tabpanel", { name: "Records" });
    const text = (): string => panel.elements()[0]?.textContent ?? "";
    await expect.poll(text).toContain("This engine's Product does not carry what was asked.");
    expect(text()).toContain("Run it with Foxhound, which contains everything Collie does");
    expect(text()).toContain("not_in_product");
    expect(called(adapter)).toEqual(expect.arrayContaining(["run.records", "run.seeds"]));
    client.close();
  },
);

test(
  "a Collie run's page shows no Foxhound view",
  { tags: ["story:12-121", "screen:/runs/$run"] },
  async () => {
    const adapter = engineOf();
    const client = clientOf(adapter);

    await openRunPage(client);

    await expect.poll(facts).toHaveProperty("Lifecycle", "running");
    await expect.element(page.getByRole("tablist")).not.toBeInTheDocument();
    expect(called(adapter)).not.toContain("run.records");
    client.close();
  },
);

test(
  "an ended run's page names the Product it needs and no owning engine",
  { tags: ["story:12-120", "screen:/runs/$run"] },
  async () => {
    const client = clientOf(
      engineOf({
        row: rowOf({ lifecycle: "finished", verdict: "verified", completeness: "complete" }),
        progress: progressOf({ lifecycle: "finished", writer: "none", last: true }),
      }),
    );

    await openRunPage(client);

    await expect.poll(facts).toMatchObject({
      "Product it needs": "Collie",
      "Owning engine": "none: no engine works this run",
    });
    client.close();
  },
);
