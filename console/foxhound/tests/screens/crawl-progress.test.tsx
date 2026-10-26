// A crawl run's Progress view (spec #593, story 217; ADRs 0102 and 0118),
// against the in-memory adapter of Foxhound's engine: it follows the run
// through `run.progress` and shows the crawl's parts of each value, the
// requests in flight with their phase, tier and attempt, why due requests
// wait per reason with the domains held, when each is next admitted and the
// setting behind the wait, each sitemap seed's progress, whether a
// keep-alive crawl is idle, its handlers against their concurrency, the
// Resource governor's level and the program's status message.
import type {
  RunProgress,
  RunProgressParams,
  SettingLimit,
} from "@goodfetch/collie-client/generated";
import type { ConsoleClient, ConsoleOperations, PageVisibility } from "@goodfetch/console/client";
import { createConsoleClient } from "@goodfetch/console/client";
import type { RunView } from "@goodfetch/console/console-build";
import { ConsoleI18n } from "@goodfetch/console/i18n";
import {
  createProtocolAdapter,
  failure,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import type { FoxhoundConsoleOperations } from "#generated/operations.ts";
import type { CrawlParts } from "#runs/crawl-parts.ts";
import { FOXHOUND_RUN_VIEWS } from "#runs/run-views.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

const RUN_ID = "crawl-run";

/** The operations a crawl run's page calls: every Run's, with Foxhound's own. */
type CrawlPageOperations = ConsoleOperations & FoxhoundConsoleOperations;

function clientOf(adapter: ProtocolAdapter<CrawlPageOperations>): ConsoleClient {
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "foxhound",
  });
}

function progressView(): RunView {
  const view = FOXHOUND_RUN_VIEWS.find((candidate) => candidate.id === "progress");
  if (view === undefined) {
    throw new Error("Foxhound's build has no Progress view");
  }
  return view;
}

/** A setting's value as the engine sends it. */
function settingValue(value: number): SettingLimit["value"] {
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a setting's value is any JSON value, which the generated type writes as an object.
  return value as unknown as SettingLimit["value"];
}

/** A crawl's progress at `position`, with `parts` beside every Run's. */
function progressOf(
  position: number,
  parts: CrawlParts & Pick<RunProgress, "message">,
  last = false,
): RunProgress {
  return {
    run_id: RUN_ID,
    position,
    at: "2026-10-24T09:00:00Z",
    lifecycle: last ? "finished" : "running",
    writer: last ? "none" : "live",
    epoch: 1,
    pauses: [],
    last,
    ...parts,
  };
}

/** The values `run.progress` gives after the first, each sent once the view waits for it. */
class ProgressFeed {
  first: RunProgress;
  readonly #waiting: ((next: RunProgress) => void)[] = [];
  readonly #senders: (() => void)[] = [];
  /** Each `after` the view read with. */
  readonly afters: number[] = [];
  /** Each Run ID the view read. */
  readonly runIds = new Set<string>();

  constructor(first: RunProgress) {
    this.first = first;
  }

  readonly answer = (params: RunProgressParams): RunProgress | Promise<RunProgress> => {
    this.runIds.add(params.run_id);
    if (params.after === undefined || params.after === null) {
      return this.first;
    }
    this.afters.push(params.after);
    return new Promise((resolve) => {
      this.#waiting.push(resolve);
      for (const sender of this.#senders.splice(0)) {
        sender();
      }
    });
  };

  /** Answers the view's waiting read with `next`, once it waits. */
  async send(next: RunProgress): Promise<void> {
    while (this.#waiting.length === 0) {
      await new Promise<void>((resolve) => this.#senders.push(resolve));
    }
    this.first = next;
    this.#waiting.shift()?.(next);
  }
}

async function show(feed: ProgressFeed | ReturnType<typeof failure>): Promise<ConsoleClient> {
  const adapter = createProtocolAdapter<CrawlPageOperations>({ product: "foxhound" });
  adapter.answer("run.progress", feed instanceof ProgressFeed ? feed.answer : feed);
  const client = clientOf(adapter);
  await render(<ConsoleI18n>{progressView().content({ client, runId: RUN_ID })}</ConsoleI18n>);
  return client;
}

/** The text of each body row of the table named `name`. */
function rowsOf(name: string): string[][] {
  const table = page.getByRole("table", { name }).element();
  return [...table.querySelectorAll("tbody tr")].map((row) =>
    [...row.querySelectorAll("th, td")].map((cell) => cell.textContent.trim()),
  );
}

/** The value of each term of the description list named `name`. */
function factsOf(name: string): Record<string, string> {
  const list = page.getByRole("region", { name }).element();
  const terms = [...list.querySelectorAll("dt")];
  return Object.fromEntries(
    terms.map((term) => [
      term.textContent.trim(),
      term.nextElementSibling?.textContent.trim() ?? "",
    ]),
  );
}

const WAITING: CrawlParts = {
  in_flight: [
    {
      request_id: "r-1",
      url: "http://pacing.test/articles/1",
      phase: "fetching",
      tier: "http",
      attempt: 1,
      started_at: "2026-10-24T08:59:58Z",
    },
    {
      request_id: "r-2",
      url: "http://shop.test/",
      phase: "waiting_for_permit",
      attempt: 2,
      started_at: "2026-10-24T08:59:59Z",
    },
  ],
  waiting: [
    {
      reason: "domain_delay",
      count: 3,
      domains: [{ domain: "pacing.test", next_admitted_at: "2026-10-24T09:00:02Z" }],
      setting: { key: "politeness.domain.delay", value: settingValue(2000), layer: "run" },
    },
    {
      reason: "domain_concurrency",
      count: 12,
      domains: [{ domain: "shop.test" }],
      more_domains: 4,
      setting: { key: "politeness.domain.concurrency", value: settingValue(1), layer: "default" },
    },
    { reason: "governor", count: 2, domains: [] },
    { reason: "a_newer_reason", count: 1, domains: [] },
  ],
};

test(
  "shows each request in flight with its phase, tier and attempt, and why due requests wait, per reason",
  { tags: ["story:12-217"] },
  async () => {
    const client = await show(new ProgressFeed(progressOf(1, WAITING)));

    await expect.element(page.getByRole("table", { name: "Requests in flight" })).toBeVisible();
    expect(rowsOf("Requests in flight")).toEqual([
      ["http://pacing.test/articles/1", "Fetching", "http", "1", expect.any(String)],
      ["http://shop.test/", "Waiting for a navigation permit", "", "2", expect.any(String)],
    ]);
    expect(
      page
        .getByRole("table", { name: "Requests in flight" })
        .element()
        .querySelector("time")
        ?.getAttribute("datetime"),
    ).toBe("2026-10-24T08:59:58Z");

    const delay = page.getByRole("region", { name: "Domain delay: 3 requests wait" });
    await expect.element(delay).toBeVisible();
    expect(rowsOf("Domains Domain delay holds")).toEqual([["pacing.test", expect.any(String)]]);
    expect(delay.element().querySelector("time")?.getAttribute("datetime")).toBe(
      "2026-10-24T09:00:02Z",
    );
    await expect
      .element(delay.getByText("Set by politeness.domain.delay: 2000, from Run"))
      .toBeVisible();

    const concurrency = page.getByRole("region", { name: "Domain concurrency: 12 requests wait" });
    expect(rowsOf("Domains Domain concurrency holds")).toEqual([
      ["shop.test", "Once a request in flight ends"],
    ]);
    await expect.element(concurrency.getByText("and 4 more domains")).toBeVisible();
    await expect
      .element(
        concurrency.getByText("Set by politeness.domain.concurrency: 1, from Built-in default"),
      )
      .toBeVisible();

    const governor = page.getByRole("region", { name: "Resource governor: 2 requests wait" });
    await expect.element(governor.getByText("No setting sets this wait.")).toBeVisible();
    await expect
      .element(page.getByRole("region", { name: "a_newer_reason: 1 request waits" }))
      .toBeVisible();
    client.close();
  },
);

test(
  "shows each sitemap seed's progress, a keep-alive crawl's idle state, its handlers, the governor's level and the status message",
  { tags: ["story:12-217"] },
  async () => {
    const client = await show(
      new ProgressFeed(
        progressOf(1, {
          message: "Reading the catalog",
          idle: true,
          in_flight: [],
          waiting: [],
          sitemaps: [
            {
              seed: "http://sitemap-seed.test/robots.txt",
              read: 3,
              failed: 1,
              failures: [],
              added: 40,
              not_added: 2,
              not_added_by_reason: [{ name: "duplicate", count: 2 }],
            },
          ],
          concurrency: {
            handlers: 3,
            handler_concurrency: 16,
            governor: "pressure",
            open_pages: 2,
            page_allowance: 4,
          },
        }),
      ),
    );

    await expect.element(page.getByText("Reading the catalog")).toBeVisible();
    await expect
      .element(
        page.getByText("Idle: the crawl waits for new work, held running by crawl.keep_alive."),
      )
      .toBeVisible();
    await expect.element(page.getByText("No request is in flight.")).toBeVisible();
    await expect.element(page.getByText("No due request waits.")).toBeVisible();
    expect(rowsOf("Sitemap seeds")).toEqual([
      ["http://sitemap-seed.test/robots.txt", "3", "1", "40", "2"],
    ]);
    expect(factsOf("Concurrency")).toEqual({
      "Handlers running": "3 of 16",
      "Resource governor": "Pressure",
      "Browser pages open": "2 of 4 allowed",
    });
    client.close();
  },
);

test(
  "follows the run, showing each value as it comes, until the last",
  { tags: ["story:12-217"] },
  async () => {
    const feed = new ProgressFeed(
      progressOf(1, { ...WAITING, concurrency: { handlers: 1, handler_concurrency: 2 } }),
    );
    const client = await show(feed);
    await expect.element(page.getByRole("cell", { name: "Fetching" })).toBeVisible();
    expect(factsOf("Concurrency")).toEqual({
      "Handlers running": "1 of 2",
      "Resource governor": "Not sampled yet",
    });

    await feed.send(
      progressOf(4, {
        message: "Handling the first article",
        in_flight: [
          {
            request_id: "r-1",
            url: "http://pacing.test/articles/1",
            phase: "handling",
            tier: "http",
            attempt: 1,
            started_at: "2026-10-24T08:59:58Z",
          },
        ],
        waiting: [],
        concurrency: { handlers: 2, handler_concurrency: 2, governor: "normal" },
      }),
    );
    await expect.element(page.getByRole("cell", { name: "Handling" })).toBeVisible();
    await expect.element(page.getByText("Handling the first article")).toBeVisible();
    await expect.element(page.getByText("No due request waits.")).toBeVisible();
    expect(factsOf("Concurrency")).toEqual({
      "Handlers running": "2 of 2",
      "Resource governor": "Normal",
    });

    await feed.send(progressOf(9, { idle: true, in_flight: [], waiting: [] }));
    await expect
      .element(
        page.getByText("Idle: the crawl waits for new work, held running by crawl.keep_alive."),
      )
      .toBeVisible();

    await feed.send(progressOf(10, { in_flight: [], waiting: [] }, true));
    await expect
      .element(page.getByText("The crawl ended: nothing is in flight or waits any more."))
      .toBeVisible();
    expect(feed.afters).toEqual([1, 4, 9]);
    expect([...feed.runIds]).toEqual([RUN_ID]);
    client.close();
  },
);

test("a run the engine does not know shows the refusal", { tags: ["story:12-217"] }, async () => {
  const client = await show(failure("run_not_found", { run_id: RUN_ID }));

  await expect.element(page.getByText("run_not_found", { exact: false })).toBeVisible();
  client.close();
});
