// "Needs you" against the in-memory adapter of the protocol (spec #592, seam
// 1; ADRs 0031 and 0122): the side menu counts what waits for a person and
// follows the list live, as `needs_you.changed` reports each item appearing
// and leaving; the screen shows each storage entry with its folder; a
// person dismisses a stopped run; and a tab that reconnects misses no item,
// resuming after the last event it read or replacing the list with a
// resync's snapshot. A person raises the Limit that stopped a run (ADRs
// 0031 and 0041): an SDK script's run then waits for its script, with its
// resume command to copy, and a Recipe's run resumes in the Daemon.
import type { LimitUsage, NeedsYouItem } from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  failure,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
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

/**
 * The adapter over every operation the Console calls, those kept off the
 * SDKs' layer such as the person-only `usage.raise` included: its default,
 * the SDK's operation table, leaves them out.
 */
type ConsoleAdapter = ProtocolAdapter<ConsoleOperations>;

/** An adapter standing for Collie's engine, as the Console reaches it. */
function consoleAdapter(): ConsoleAdapter {
  return createProtocolAdapter<ConsoleOperations>();
}

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

const FOLDER = "/home/ada/.goodfetch";

const LOW: NeedsYouItem = {
  id: `storage_level:${FOLDER}`,
  kind: "storage_level",
  since: "2026-10-24T09:00:00Z",
  summary: `The filesystem holding ${FOLDER} is at the low storage level.`,
  actions: [],
  details: { level: "low", folder: FOLDER },
};

const STOPPED: NeedsYouItem = {
  id: "run_stopped_by_storage:nightly:2026-10-24T09:05:00Z",
  kind: "run_stopped_by_storage",
  since: "2026-10-24T09:05:00Z",
  run: "nightly",
  summary: "The Run nightly stopped.",
  actions: ["needs_you.dismiss"],
  details: { folder: `${FOLDER}/1/runs/nightly` },
};

/** The Console's Client over `adapter`. */
function clientOf(adapter: ConsoleAdapter): ConsoleClient {
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "collie",
  });
}

async function shown(client: ConsoleClient): Promise<void> {
  await render(
    <ConsoleI18n>
      <Shell client={client} route="/needs-you" />
    </ConsoleI18n>,
  );
}

/** The side menu's link to "Needs you". */
function menuLink() {
  return page.getByRole("navigation", { name: "Console" }).getByRole("link", { name: /Needs you/ });
}

/** The list of what needs a person. */
function items() {
  return page.getByRole("list", { name: "Needs you" }).getByRole("listitem");
}

async function subscribed(adapter: ConsoleAdapter): Promise<void> {
  await expect
    .poll(() => adapter.requests.some((request) => request.method === "events.subscribe"))
    .toBe(true);
}

test(
  "the side menu counts what needs you and follows the list live, each storage entry shown with its folder",
  { tags: ["story:12-23", "story:12-25", "story:12-32", "screen:/needs-you"] },
  async () => {
    const adapter = consoleAdapter();
    adapter.answer("needs_you.list", { items: [LOW] });
    const client = clientOf(adapter);
    await shown(client);

    await expect.element(menuLink()).toHaveTextContent("Needs you1");
    await expect.element(items().nth(0)).toMatchTextContent(FOLDER);
    await expect.element(items().nth(0)).toMatchTextContent("low");
    await subscribed(adapter);

    adapter.push({ event: "needs_you.changed", data: { change: "listed", item: STOPPED } }, 1);

    await expect.element(menuLink()).toHaveTextContent("Needs you2");
    await expect.element(items().nth(1)).toMatchTextContent("nightly");
    await expect.element(items().nth(1)).toMatchTextContent(`${FOLDER}/1/runs/nightly`);

    adapter.push(
      {
        event: "needs_you.changed",
        data: { change: "removed", id: LOW.id, reason: "resolved" },
      },
      2,
    );

    await expect.element(menuLink()).toHaveTextContent("Needs you1");
    await expect.poll(() => items().elements().length).toBe(1);
    await expect.element(items().nth(0)).toMatchTextContent("nightly");
    client.close();
  },
);

test(
  "a person dismisses a stopped run, and the change the engine publishes takes it off the list",
  { tags: ["story:12-32", "screen:/needs-you"] },
  async () => {
    const adapter = consoleAdapter();
    adapter.answer("needs_you.list", { items: [LOW, STOPPED] });
    adapter.answer("needs_you.dismiss", ({ item }) => {
      adapter.push(
        { event: "needs_you.changed", data: { change: "removed", id: item, reason: "dismissed" } },
        1,
      );
      return { dismissed: true };
    });
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    await expect
      .element(items().nth(0).getByRole("button", { name: "Dismiss" }))
      .not.toBeInTheDocument();
    await items().nth(1).getByRole("button", { name: "Dismiss" }).click();

    await expect.poll(() => items().elements().length).toBe(1);
    await expect.element(menuLink()).toHaveTextContent("Needs you1");
    expect(
      adapter.requests
        .filter((request) => request.method === "needs_you.dismiss")
        .map((request) => request.params),
    ).toEqual([{ item: STOPPED.id }]);
    client.close();
  },
);

test(
  "a tab that reconnects misses no item: it resumes after its last event, or takes a resync's snapshot",
  { tags: ["story:12-23", "screen:/needs-you"] },
  async () => {
    const adapter = consoleAdapter();
    adapter.answer("needs_you.list", { items: [] });
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);
    await expect.element(page.getByText("Nothing needs you now.")).toBeVisible();

    adapter.push({ event: "needs_you.changed", data: { change: "listed", item: LOW } }, 1);
    await expect.element(menuLink()).toHaveTextContent("Needs you1");
    adapter.drop();
    adapter.push({ event: "needs_you.changed", data: { change: "listed", item: STOPPED } }, 2);

    await expect.element(menuLink()).toHaveTextContent("Needs you2");

    adapter.restart();
    adapter.push(
      {
        event: "needs_you.changed",
        data: {
          change: "listed",
          item: { ...LOW, details: { level: "critical", folder: FOLDER } },
        },
      },
      1,
    );

    await expect.element(menuLink()).toHaveTextContent("Needs you1");
    await expect.poll(() => items().elements().length).toBe(1);
    await expect.element(items().nth(0)).toMatchTextContent("critical");
    client.close();
  },
);

/** The install's monthly Limit on requests, as `limit_reached` names it. */
const MONTHLY = {
  scope: { kind: "install", id: "" },
  usage_kind: "requests",
  period: { every: "month", time_zone: "Europe/Paris" },
  resets_at: "2026-10-31T23:00:00Z",
} as const;

/** The run `capped`, stopped by the install's monthly Limit of 1,000 requests; a Recipe's run when `recipe` names one. */
function stoppedByLimit(recipe?: string): NeedsYouItem {
  return {
    id: "run_stopped_by_limit:capped:2026-10-24T09:10:00Z",
    kind: "run_stopped_by_limit",
    since: "2026-10-24T09:10:00Z",
    run: "capped",
    summary: "The Run capped stopped: a Limit on requests has nothing left.",
    actions:
      recipe === undefined
        ? ["usage.raise", "needs_you.dismiss"]
        : ["usage.raise", "run.resume", "needs_you.dismiss"],
    details: {
      limit: MONTHLY,
      amount: 1000,
      needs: "collie",
      ...(recipe === undefined ? {} : { recipe }),
    },
  };
}

/** What `usage.raise` answers for the install's monthly Limit raised to `amount`. */
function raisedTo(amount: number): LimitUsage {
  return {
    limit: {
      scope: MONTHLY.scope,
      usage_kind: MONTHLY.usage_kind,
      period: MONTHLY.period,
      amount,
      must_understand: true,
      set_by: "person",
    },
    charged: 1000,
    held: 0,
    left: amount - 1000,
    period_started_at: "2026-09-30T22:00:00Z",
    resets_at: MONTHLY.resets_at,
    standing: "held",
  };
}

/** An engine listing `item` that raises its Limit, then takes it off the list as resolved. */
function raising(item: NeedsYouItem): ConsoleAdapter {
  const adapter = consoleAdapter();
  adapter.answer("needs_you.list", { items: [item] });
  adapter.answer("usage.raise", ({ amount }) => {
    adapter.push(
      { event: "needs_you.changed", data: { change: "removed", id: item.id, reason: "resolved" } },
      1,
    );
    return raisedTo(amount);
  });
  return adapter;
}

/** Raises the shown item's Limit to `amount`, as a person types it. */
async function raise(amount: string): Promise<void> {
  const field = page.getByRole("textbox", { name: "New amount per period" });
  await userEvent.clear(field);
  await userEvent.type(field, amount);
  await userEvent.click(page.getByRole("button", { name: "Raise the Limit" }));
}

/** The calls of `method` the Console made, by their params. */
function calls(adapter: ConsoleAdapter, method: string): unknown[] {
  return adapter.requests
    .filter((request) => request.method === method)
    .map((request) => request.params);
}

test(
  "a person raises the Limit that stopped an SDK script's run, which then waits for its script with its resume command to copy",
  { tags: ["story:12-100", "story:12-32", "screen:/needs-you"] },
  async () => {
    const adapter = raising(stoppedByLimit());
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    await expect.element(items().nth(0)).toMatchTextContent("The run capped stopped");
    await expect.element(items().nth(0)).toMatchTextContent("requests");
    await expect.element(items().nth(0)).toMatchTextContent("Europe/Paris");
    await raise("900");
    await expect
      .element(page.getByRole("alert"))
      .toHaveTextContent("Enter an amount above the 1,000 it allows now.");
    expect(calls(adapter, "usage.raise")).toEqual([]);

    await raise("5000");

    await expect.poll(() => items().elements().length).toBe(0);
    await expect.element(menuLink()).not.toMatchTextContent(/\d/);
    expect(calls(adapter, "usage.raise")).toEqual([
      {
        scope: "install",
        scope_id: "",
        usage_kind: "requests",
        period: MONTHLY.period,
        amount: 5000,
      },
    ]);
    expect(calls(adapter, "run.resume")).toEqual([]);
    await expect
      .element(page.getByRole("status").filter({ hasText: "raised" }))
      .toHaveTextContent(
        "You raised the Limit. The run capped waits for its script: start the program again with its Run ID to resume it.",
      );
    await expect
      .element(page.getByText("goodfetch-collie run resume capped", { exact: true }))
      .toBeVisible();
    // The component project's browser grants no clipboard permission, so the
    // copy is refused here and the screen says how to copy it by hand.
    await userEvent.click(page.getByRole("button", { name: "Copy resume command" }));
    await expect
      .element(page.getByRole("status").filter({ hasText: "copy" }))
      .toHaveTextContent("The browser refused to copy the resume command: select it and copy it.");
    client.close();
  },
);

test(
  "a person raises the Limit that stopped a Recipe's run, which then resumes in the Daemon",
  { tags: ["story:12-100", "screen:/needs-you"] },
  async () => {
    const adapter = raising(stoppedByLimit("nightly-prices"));
    adapter.answer("run.resume", {
      run_id: "capped",
      started_as: "resumed",
      lifecycle: "running",
      needs: "collie",
    });
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);
    await expect.element(items().nth(0)).toMatchTextContent("nightly-prices");

    await raise("5000");

    await expect.poll(() => calls(adapter, "run.resume")).toEqual([{ run_id: "capped" }]);
    await expect
      .element(page.getByRole("status").filter({ hasText: "raised" }))
      .toHaveTextContent("You raised the Limit, and the run capped resumes in the Daemon.");
    expect(page.getByRole("button", { name: "Copy resume command" }).elements()).toHaveLength(0);
    client.close();
  },
);

test(
  "the engine's refusal to raise a Limit is shown, and the run stays listed",
  { tags: ["story:12-100", "screen:/needs-you"] },
  async () => {
    const adapter = consoleAdapter();
    adapter.answer("needs_you.list", { items: [stoppedByLimit()] });
    adapter.answer("usage.raise", failure("person_required", {}));
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    await raise("5000");

    await expect
      .element(page.getByRole("alert"))
      .toHaveTextContent(
        "The engine did not raise the Limit: The test answered usage.raise with person_required.",
      );
    await expect.poll(() => items().elements().length).toBe(1);
    client.close();
  },
);
