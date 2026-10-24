// "Needs you" against the in-memory adapter of the protocol (spec #592, seam
// 1; ADRs 0031 and 0122): the side menu counts what waits for a person and
// follows the list live, as `needs_you.changed` reports each item appearing
// and leaving; the screen shows each storage entry with its folder; a
// person dismisses a stopped run; a tab that reconnects misses no item,
// resuming after the last event it read or replacing the list with a
// resync's snapshot; and a domain that showed a consent wall is offered for
// `consent.domains`, alone or as `*`, each with the warning of what the
// person takes on, answered in one click.
import type { NeedsYouItem } from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  failure,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { createConsoleClient, type ConsoleClient, type PageVisibility } from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import type { ConsoleOperations } from "#generated/operations.ts";
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
function clientOf(adapter: Pick<ProtocolAdapter, "transport" | "bodyFetcher">): ConsoleClient {
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

async function subscribed(adapter: Pick<ProtocolAdapter, "requests">): Promise<void> {
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

/** The in-memory adapter answering the Console's operations, a person's answers among them. */
type ConsoleAdapter = ProtocolAdapter<ConsoleOperations>;

const WALL: NeedsYouItem = {
  id: "consent_wall:walled.test",
  kind: "consent_wall",
  since: "2026-10-24T09:10:00Z",
  run: "nightly",
  summary: "walled.test showed a consent wall.",
  actions: ["needs_you.allow_consent_domain", "needs_you.dismiss"],
  details: { domain: "walled.test" },
};

/** Answers `needs_you.allow_consent_domain`, taking the wall off the list as the engine does. */
function allowing(adapter: ConsoleAdapter): void {
  adapter.answer("needs_you.allow_consent_domain", ({ domain }) => {
    adapter.push(
      { event: "needs_you.changed", data: { change: "removed", id: WALL.id, reason: "resolved" } },
      1,
    );
    return { domain, added: true, layer: "user_configuration", warning: "" };
  });
}

/** The params of each `needs_you.allow_consent_domain` call. */
function allowed(adapter: ConsoleAdapter): unknown[] {
  return adapter.requests
    .filter((request) => request.method === "needs_you.allow_consent_domain")
    .map((request) => request.params);
}

test(
  "a domain that showed a consent wall is offered for consent.domains with what the person takes on, answered in one click",
  { tags: ["story:12-26", "screen:/needs-you", "operation:needs_you.allow_consent_domain"] },
  async () => {
    const adapter = createProtocolAdapter<ConsoleOperations>();
    adapter.answer("needs_you.list", { items: [WALL] });
    allowing(adapter);
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    const wall = items().nth(0);
    await expect
      .element(wall.getByRole("heading"))
      .toHaveTextContent("walled.test showed a consent wall");
    await expect.element(wall).toMatchTextContent(/consent in your name/);
    await wall
      .getByRole("button", { name: "Accept walled.test's consent walls in later runs" })
      .click();

    await expect.poll(() => items().elements().length).toBe(0);
    await expect.element(page.getByText("Nothing needs you now.")).toBeVisible();
    expect(page.getByRole("alertdialog").elements()).toHaveLength(0);
    expect(allowed(adapter)).toEqual([{ domain: "walled.test" }]);
    client.close();
  },
);

test(
  "accepting consent walls on every domain shows its own warning before the one click that writes *",
  { tags: ["story:12-26", "screen:/needs-you"] },
  async () => {
    const adapter = createProtocolAdapter<ConsoleOperations>();
    adapter.answer("needs_you.list", { items: [WALL] });
    allowing(adapter);
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    const wall = items().nth(0);
    await expect.element(wall).toMatchTextContent(/every site that shows one/);
    await expect.element(wall).toMatchTextContent(/no longer asks you/);
    await wall.getByRole("button", { name: "Accept consent walls on every domain" }).click();

    await expect.poll(() => items().elements().length).toBe(0);
    expect(allowed(adapter)).toEqual([{ domain: "*" }]);
    client.close();
  },
);

test(
  "an answer the engine refuses, such as under an Operator policy's lock, is said and leaves the item listed",
  { tags: ["story:12-26", "screen:/needs-you"] },
  async () => {
    const adapter = createProtocolAdapter<ConsoleOperations>();
    adapter.answer("needs_you.list", { items: [WALL] });
    adapter.answer(
      "needs_you.allow_consent_domain",
      failure("setting_locked", {
        key: "consent.domains",
        layer: "user_configuration",
        limit: { key: "consent.domains", value: [], layer: "operator_policy" },
      }),
    );
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    await items()
      .nth(0)
      .getByRole("button", { name: "Accept walled.test's consent walls in later runs" })
      .click();

    await expect.element(page.getByRole("alert")).toMatchTextContent(/could not add the domain/);
    await expect.poll(() => items().elements().length).toBe(1);
    client.close();
  },
);
