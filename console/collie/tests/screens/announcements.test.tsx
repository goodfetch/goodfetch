// What the Console announces (ADR 0073), heard through the virtual screen
// reader over the in-memory adapter of the protocol: "Needs you" as its
// count changes, an action's notice, as a toast would say it, an error the
// engine answers an action with, and a run's change of Lifecycle. Each is
// asserted by the live region's politeness and the words a person hears,
// never by formatted values (TESTING.md, Localization).
import type {
  NeedsYouItem,
  RunIndexRow,
  RunProgress,
  RunProgressParams,
  RunUsage,
} from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  failure,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { virtualScreenReader } from "@goodfetch/test-support/vitest-browser";
import { expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import {
  createConsoleClient,
  type ConsoleClient,
  type ConsoleOperations,
  type PageVisibility,
} from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

const RUN_ID = "nightly";

const LOW: NeedsYouItem = {
  id: "storage_level:/home/ada/.goodfetch",
  kind: "storage_level",
  since: "2026-10-24T09:00:00Z",
  summary: "The filesystem holding /home/ada/.goodfetch is at the low storage level.",
  actions: [],
  details: { level: "low", folder: "/home/ada/.goodfetch" },
};

const STOPPED: NeedsYouItem = {
  id: "run_stopped_by_storage:nightly:2026-10-24T09:05:00Z",
  kind: "run_stopped_by_storage",
  since: "2026-10-24T09:05:00Z",
  run: RUN_ID,
  summary: "The Run nightly stopped.",
  actions: ["needs_you.dismiss"],
  details: { folder: "/home/ada/.goodfetch/1/runs/nightly" },
};

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

const USAGE: RunUsage = {
  browser_time: 0,
  model: { calls: 0, tokens: 0 },
  proxy_bytes: 0,
  requests: 0,
};

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

/** `run.progress`'s answers: the first at once, each later one sent once the page waits for it. */
class ProgressFeed {
  #current: RunProgress;
  readonly #waiting: ((next: RunProgress) => void)[] = [];
  readonly #senders: (() => void)[] = [];

  constructor(first: RunProgress) {
    this.#current = first;
  }

  readonly answer = (params: RunProgressParams): RunProgress | Promise<RunProgress> =>
    params.after === undefined || params.after === null
      ? this.#current
      : new Promise((resolve) => {
          this.#waiting.push(resolve);
          for (const sender of this.#senders.splice(0)) {
            sender();
          }
        });

  async send(next: RunProgress): Promise<void> {
    while (this.#waiting.length === 0) {
      await new Promise<void>((resolve) => this.#senders.push(resolve));
    }
    this.#current = next;
    this.#waiting.shift()?.(next);
  }
}

/** The engine of a live run, its progress from `feed` and its Run index row from `row`. */
function runEngine(
  feed: ProgressFeed,
  row: () => RunIndexRow = () => rowOf(),
): ProtocolAdapter<ConsoleOperations> {
  const adapter = createProtocolAdapter<ConsoleOperations>();
  adapter.answer("needs_you.list", { items: [] });
  adapter.answer("run.progress", feed.answer);
  adapter.answer("run.usage", USAGE);
  adapter.answer("run.list", () => ({ runs: [row()] }));
  adapter.answer("run.report", () => {
    const { lifecycle, verdict, completeness } = row();
    return {
      run_id: RUN_ID,
      lifecycle,
      verdict: verdict ?? "verified",
      completeness: completeness ?? "complete",
      started_at: "2026-10-24T08:00:00Z",
      ended_at: "2026-10-24T09:10:00Z",
      usage: USAGE,
    };
  });
  return adapter;
}

function clientOf(adapter: ProtocolAdapter<ConsoleOperations>): ConsoleClient {
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "collie",
  });
}

async function shellAt(client: ConsoleClient, route: string): Promise<void> {
  await render(
    <ConsoleI18n>
      <Shell client={client} route={route} />
    </ConsoleI18n>,
  );
}

/** What the reader announced from live regions, in order. */
async function announced(reader: Awaited<ReturnType<typeof virtualScreenReader>>) {
  return (await reader.spokenPhraseLog()).filter((phrase) => /^(polite|assertive): /u.test(phrase));
}

test(
  '"Needs you" announces how many items wait as an item arrives and leaves',
  { tags: ["story:12-131", "screen:/needs-you"] },
  async () => {
    const adapter = createProtocolAdapter<ConsoleOperations>();
    adapter.answer("needs_you.list", { items: [LOW] });
    adapter.answer("run.list", { runs: [] });
    const client = clientOf(adapter);
    const reader = await virtualScreenReader();
    await shellAt(client, "/runs");
    await expect
      .poll(() => adapter.requests.some((request) => request.method === "events.subscribe"))
      .toBe(true);

    adapter.push({ event: "needs_you.changed", data: { change: "listed", item: STOPPED } }, 1);
    await expect.poll(() => announced(reader)).toContain("polite: 2 items need you");
    adapter.push(
      { event: "needs_you.changed", data: { change: "removed", id: LOW.id, reason: "resolved" } },
      2,
    );

    await expect.poll(() => announced(reader)).toContain("polite: 1 item needs you");
    client.close();
  },
);

test(
  "a run's page announces an action's notice, and the error an action is refused with",
  { tags: ["story:12-131", "screen:/runs/$run"] },
  async () => {
    const adapter = runEngine(new ProgressFeed(progressOf()));
    adapter.answer("run.checkpoint", {
      run_id: RUN_ID,
      digest: "sha256:0f",
      journal_position: 4,
      location: "/srv/goodfetch/runs/nightly/checkpoint.sqlite",
      run_epoch: 1,
      size: 4096,
      time: "2026-10-24T09:01:00Z",
    });
    adapter.answer("run.stop", failure("run_ended", { run_id: RUN_ID }));
    const client = clientOf(adapter);
    await shellAt(client, `/runs/${RUN_ID}`);
    const reader = await virtualScreenReader();

    await userEvent.click(page.getByRole("button", { name: "Write a Checkpoint" }));
    await expect
      .poll(async () => (await announced(reader)).join("\n"))
      .toMatch(/^polite: Checkpoint written at /mu);
    await userEvent.click(page.getByRole("button", { name: "Stop" }));

    await expect
      .poll(async () => (await announced(reader)).join("\n"))
      .toMatch(/^polite: The engine refused:/mu);
    client.close();
  },
);

test(
  "a run's page announces its new Lifecycle when the run ends",
  { tags: ["story:12-131", "screen:/runs/$run"] },
  async () => {
    const feed = new ProgressFeed(progressOf());
    let row = rowOf();
    const client = clientOf(runEngine(feed, () => row));
    await shellAt(client, `/runs/${RUN_ID}`);
    await expect.element(page.getByRole("button", { name: "Stop" })).toBeVisible();
    const reader = await virtualScreenReader();

    row = rowOf({ lifecycle: "finished", verdict: "verified", completeness: "complete" });
    await feed.send(progressOf({ position: 2, lifecycle: "finished", writer: "none", last: true }));

    await expect.poll(() => announced(reader)).toContain("polite: The run is finished.");
    client.close();
  },
);
