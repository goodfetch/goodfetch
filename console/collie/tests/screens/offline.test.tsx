// The screens over the offline cache (spec #592's `offline`; ADR 0031),
// against the in-memory adapter of the protocol and a cache kept in memory:
// the Runs screen keeps each run it lists, without the text a program
// writes; and a screen that opens with no Connection, as a page moving
// between screens while the Daemon is stopped, shows what the cache keeps:
// the Runs screen its runs, newest first, saying the list is this browser's
// copy, and a run's page its run.
import type { RunIndexRow, RunProgressParams } from "@goodfetch/collie-client/generated";
import { createProtocolAdapter, failure } from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import {
  createConsoleClient,
  type ConsoleClient,
  type PageVisibility,
  type Transport,
} from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import type { OfflineCache, OfflineKind } from "#offline/offline.ts";
import { Shell } from "#shell.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

/** An offline cache kept in memory. */
class MemoryCache implements OfflineCache {
  readonly rows = new Map<string, unknown>();

  async put(kind: OfflineKind, id: string, value: unknown): Promise<void> {
    this.rows.set(`${kind}/${id}`, structuredClone(value));
    return Promise.resolve();
  }

  async get(kind: OfflineKind, id: string): Promise<unknown> {
    return Promise.resolve(this.rows.get(`${kind}/${id}`));
  }

  async all(kind: OfflineKind): Promise<unknown[]> {
    return Promise.resolve(
      [...this.rows].filter(([name]) => name.startsWith(`${kind}/`)).map(([, value]) => value),
    );
  }

  async forgetDataset(): Promise<void> {
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

function clientOver(transport: Transport): ConsoleClient {
  return createConsoleClient({
    transport,
    bodyFetcher: async () => Promise.reject(new Error("no bodies offline")),
    visibility: visible,
    product: "collie",
  });
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

/** The Run IDs the table lists, in its order. */
function listedIds(): string[] {
  return page
    .getByRole("rowheader")
    .elements()
    .map((cell) => cell.querySelector("code")?.textContent ?? "");
}

test(
  "the Runs screen keeps each run it lists in the offline cache, without a program's text",
  { tags: ["story:12-126", "screen:/runs"] },
  async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("needs_you.list", { items: [] });
    adapter.answer("run.list", {
      runs: [
        row("live-run", {
          tags: ["team:shop", "goodfetch.recipe:shop-prices"],
          progress_message: "Read 1 of 3 pages",
        }),
      ],
    });
    const client = clientOver(adapter.transport);
    const cache = new MemoryCache();

    await render(
      <ConsoleI18n>
        <Shell client={client} route="/runs" offline={cache} />
      </ConsoleI18n>,
    );

    await expect
      .poll(() => cache.rows.get("run_summary/live-run"))
      .toEqual({
        run_id: "live-run",
        location: "runs/live-run",
        kind: "collie",
        needs: "collie",
        tags: ["goodfetch.recipe:shop-prices"],
        lifecycle: "running",
        verdict: null,
        completeness: null,
        started_at: "2026-10-01T08:00:00Z",
        ended_at: null,
        owner: null,
        presence: "present",
        progress_message: null,
        stop_reason: null,
        replayed: false,
      });
    client.close();
  },
);

test(
  "with no Connection, the Runs screen shows the runs the offline cache keeps, newest first, as this browser's copy",
  { tags: ["story:12-126", "screen:/runs"] },
  async () => {
    const cache = new MemoryCache();
    await cache.put("run_summary", "older-run", row("older-run"));
    await cache.put(
      "run_summary",
      "newer-run",
      row("newer-run", { lifecycle: "finished", started_at: "2026-10-02T08:00:00Z" }),
    );
    const client = clientOver(unreachable);

    await render(
      <ConsoleI18n>
        <Shell client={client} route="/runs" offline={cache} />
      </ConsoleI18n>,
    );

    await expect.poll(listedIds).toEqual(["newer-run", "older-run"]);
    await expect
      .element(page.getByText("This browser's offline copy", { exact: false }))
      .toBeVisible();
    client.close();
  },
);

test(
  "with no Connection, a run's page shows what the offline cache keeps of it",
  { tags: ["story:12-126", "screen:/runs/$run"] },
  async () => {
    const cache = new MemoryCache();
    await cache.put("run_page", "kept-run", {
      progress: {
        run_id: "kept-run",
        position: 3,
        at: "2026-10-02T09:00:00Z",
        lifecycle: "finished",
        writer: "none",
        epoch: 1,
        checkpoint_at: null,
        pauses: [],
        message: null,
        storage: null,
        stop_reason: null,
        last: true,
      },
      axes: { lifecycle: "finished", verdict: "verified", completeness: "complete" },
      needs: "collie",
      uncapped: false,
    });
    const client = clientOver(unreachable);

    await render(
      <ConsoleI18n>
        <Shell client={client} route="/runs/kept-run" offline={cache} />
      </ConsoleI18n>,
    );

    await expect.element(page.getByText("verified", { exact: true })).toBeVisible();
    await expect.element(page.getByText("complete", { exact: true })).toBeVisible();
    client.close();
  },
);

test(
  "a run's page keeps showing its run when a later read is cancelled, as when the Daemon stops",
  { tags: ["story:12-125", "screen:/runs/$run"] },
  async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("needs_you.list", { items: [] });
    adapter.answer("run.list", { runs: [row("live-run")] });
    adapter.answer("run.usage", failure("call_cancelled"));
    adapter.answer("run.progress", (params: RunProgressParams) =>
      params.after === undefined || params.after === null
        ? {
            run_id: "live-run",
            position: 1,
            at: "2026-10-01T08:05:00Z",
            lifecycle: "running",
            writer: "live",
            epoch: 1,
            pauses: [],
            last: false,
          }
        : failure("call_cancelled"),
    );
    const client = clientOver(adapter.transport);

    await render(
      <ConsoleI18n>
        <Shell client={client} route="/runs/live-run" />
      </ConsoleI18n>,
    );

    await expect
      .poll(() =>
        adapter.requests.some(
          ({ method, params }) =>
            method === "run.progress" &&
            typeof params === "object" &&
            params !== null &&
            "after" in params,
        ),
      )
      .toBe(true);
    await expect.element(page.getByText("running", { exact: true })).toBeVisible();
    expect(page.getByRole("alert").elements()).toHaveLength(0);
    client.close();
  },
);

test(
  "Sign out asks the page to sign out, and waits for a Connection",
  { tags: ["story:12-126", "screen:/runs"] },
  async () => {
    const adapter = createProtocolAdapter();
    adapter.answer("needs_you.list", { items: [] });
    adapter.answer("run.list", { runs: [] });
    const connected = clientOver(adapter.transport);
    let signedOut = 0;

    const shown = await render(
      <ConsoleI18n>
        <Shell client={connected} route="/runs" onSignOut={() => (signedOut += 1)} />
      </ConsoleI18n>,
    );
    await userEvent.click(page.getByRole("button", { name: "Sign out" }));
    expect(signedOut).toBe(1);
    connected.close();

    const disconnected = clientOver(unreachable);
    await shown.rerender(
      <ConsoleI18n>
        <Shell client={disconnected} route="/runs" onSignOut={() => (signedOut += 1)} />
      </ConsoleI18n>,
    );
    await expect.element(page.getByRole("alert")).toBeVisible();
    await expect.element(page.getByRole("button", { name: "Sign out" })).toBeDisabled();
    disconnected.close();
  },
);
