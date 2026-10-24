// "Needs you" against the in-memory adapter of the protocol (spec #592, seam
// 1; ADRs 0031 and 0122): the side menu counts what waits for a person and
// follows the list live, as `needs_you.changed` reports each item appearing
// and leaving; the screen shows each storage entry with its folder; a
// person dismisses a stopped run; and a tab that reconnects misses no item,
// resuming after the last event it read or replacing the list with a
// resync's snapshot.
import type { NeedsYouItem } from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { createConsoleClient, type ConsoleClient, type PageVisibility } from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";

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
function clientOf(adapter: ProtocolAdapter): ConsoleClient {
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

async function subscribed(adapter: ProtocolAdapter): Promise<void> {
  await expect
    .poll(() => adapter.requests.some((request) => request.method === "events.subscribe"))
    .toBe(true);
}

test(
  "the side menu counts what needs you and follows the list live, each storage entry shown with its folder",
  { tags: ["story:12-23", "story:12-25", "story:12-32", "screen:/needs-you"] },
  async () => {
    const adapter = createProtocolAdapter();
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
    const adapter = createProtocolAdapter();
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
    const adapter = createProtocolAdapter();
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
