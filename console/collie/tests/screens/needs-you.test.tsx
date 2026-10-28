// "Needs you" against the in-memory adapter of the protocol (spec #592, seam
// 1; ADRs 0031 and 0122): the side menu counts what waits for a person and
// follows the list live, as `needs_you.changed` reports each item appearing
// and leaving; the screen shows each storage entry with its folder; a
// person dismisses a stopped run; and a tab that reconnects misses no item,
// resuming after the last event it read or replacing the list with a
// resync's snapshot. A person raises the Limit that stopped a run (ADRs
// 0031 and 0041): an SDK script's run then waits for its script, with its
// resume command to copy, and a Recipe's run resumes in the Daemon. A
// domain that showed a consent wall is offered for `consent.domains`, alone
// or as `*`, each with the warning of what the person takes on, answered in
// one click. A stop of the refetch without the site's stored data (ADR 0052)
// is shown with the file that lists its domain: a person removes it there,
// replaces `*` with a list, dismisses the item or denies the domain with a
// reason, and a domain from project configuration offers no removal. A
// certificate whose key changed (ADR 0042) shows both keys, and its person
// accepts the new key or forgets the pin. Its render budget (ADR 0071) is
// the commits an item added and one resolved take, snapshotted under
// `<Profiler>`.
import type { LimitUsage, NeedsYouItem } from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  failure,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { Profiler } from "react";
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
  "the list orders items by the time their since names, not by its text, as the engine does",
  { tags: ["story:12-23", "screen:/needs-you"] },
  async () => {
    const adapter = consoleAdapter();
    const fraction: NeedsYouItem = {
      ...STOPPED,
      id: "run_stopped_by_storage:fraction",
      since: "2026-10-24T09:00:00.5Z",
      run: "fraction",
      summary: "The Run fraction stopped.",
    };
    const offset: NeedsYouItem = {
      ...STOPPED,
      id: "run_stopped_by_storage:offset",
      since: "2026-10-24T10:30:00+02:00",
      run: "offset",
      summary: "The Run offset stopped.",
    };
    adapter.answer("needs_you.list", { items: [fraction, LOW, offset] });
    const client = clientOf(adapter);
    await shown(client);

    await expect.poll(() => items().elements().length).toBe(3);
    await expect.element(items().nth(0)).toMatchTextContent("offset");
    await expect.element(items().nth(2)).toMatchTextContent("fraction");
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
  { tags: ["story:12-100", "story:12-133", "screen:/needs-you"] },
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
        "The engine did not raise the Limit:" +
          "Only a person can give this answer." +
          "Error codeperson_required",
      );
    await expect.poll(() => items().elements().length).toBe(1);
    client.close();
  },
);

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
    const adapter = consoleAdapter();
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
    const adapter = consoleAdapter();
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
    const adapter = consoleAdapter();
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

const CONFIGURATION = "/home/ada/.goodfetch/1/configuration.toml";

/** A stop of the refetch without the site's stored data on walled.test, its domain listed in `layer`'s `file`. */
function refetchStopped(layer: string, file: string, onlyStar = false): NeedsYouItem {
  return {
    id: "wall_refetch_stopped:walled.test",
    kind: "wall_refetch_stopped",
    since: "2026-10-24T09:20:00Z",
    run: "nightly",
    summary: "walled.test refused a page fetched again without its stored cookies and storage.",
    actions: [
      ...(layer === "project_configuration" ? [] : ["needs_you.remove_refetch_domain"]),
      "needs_you.deny_wall_domain",
      "needs_you.dismiss",
    ],
    details: { domain: "walled.test", cause: "challenge", layer, file, only_star: onlyStar },
  };
}

/** Takes `id` off the list, as the engine does once it took an answer. */
function resolve(adapter: ConsoleAdapter, id: string): void {
  adapter.push(
    { event: "needs_you.changed", data: { change: "removed", id, reason: "resolved" } },
    1,
  );
}

test(
  "a refetch stopped without the site's stored data is listed with the file that lists its domain, and a person removes the domain there",
  { tags: ["story:12-27", "screen:/needs-you", "operation:needs_you.remove_refetch_domain"] },
  async () => {
    const adapter = consoleAdapter();
    const item = refetchStopped("user_configuration", CONFIGURATION);
    adapter.answer("needs_you.list", { items: [item] });
    adapter.answer("needs_you.remove_refetch_domain", ({ domain }) => {
      resolve(adapter, item.id);
      return { domain, removed: true, layer: "user_configuration", domains: [] };
    });
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    const stop = items().nth(0);
    await expect
      .element(stop.getByRole("heading"))
      .toHaveTextContent("walled.test refused a page fetched again without its stored data");
    await expect.element(stop).toMatchTextContent(CONFIGURATION);
    await expect.element(stop).toMatchTextContent(/challenge/);
    await stop
      .getByRole("button", { name: "Remove walled.test from wall.refetch_domains" })
      .click();

    await expect.poll(() => items().elements().length).toBe(0);
    expect(calls(adapter, "needs_you.remove_refetch_domain")).toEqual([{ domain: "walled.test" }]);
    client.close();
  },
);

test(
  "when only * covers the domain, the person replaces * with the list they give",
  { tags: ["story:12-27", "screen:/needs-you"] },
  async () => {
    const adapter = consoleAdapter();
    const item = refetchStopped("user_configuration", CONFIGURATION, true);
    adapter.answer("needs_you.list", { items: [item] });
    adapter.answer("needs_you.remove_refetch_domain", ({ domain, instead_of_star }) => {
      resolve(adapter, item.id);
      return { domain, removed: true, layer: "user_configuration", domains: instead_of_star ?? [] };
    });
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    const stop = items().nth(0);
    expect(
      stop.getByRole("button", { name: "Remove walled.test from wall.refetch_domains" }).elements(),
    ).toHaveLength(0);
    await userEvent.type(
      stop.getByRole("textbox", { name: "Domains that replace *" }),
      "other.test, third.test",
    );
    await stop.getByRole("button", { name: "Replace * with this list" }).click();

    await expect.poll(() => items().elements().length).toBe(0);
    expect(calls(adapter, "needs_you.remove_refetch_domain")).toEqual([
      { domain: "walled.test", instead_of_star: ["other.test", "third.test"] },
    ]);
    client.close();
  },
);

test(
  "a domain project configuration lists is shown with its file, offering no removal, and a person dismisses the item",
  { tags: ["story:12-27", "screen:/needs-you", "operation:needs_you.dismiss"] },
  async () => {
    const adapter = consoleAdapter();
    const file = "/home/ada/project/goodfetch.toml";
    const item = refetchStopped("project_configuration", file);
    adapter.answer("needs_you.list", { items: [item] });
    adapter.answer("needs_you.dismiss", () => {
      adapter.push(
        {
          event: "needs_you.changed",
          data: { change: "removed", id: item.id, reason: "dismissed" },
        },
        1,
      );
      return { dismissed: true };
    });
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    const stop = items().nth(0);
    await expect.element(stop).toMatchTextContent(file);
    await expect.element(stop).toMatchTextContent(/remove it from that file/);
    expect(stop.getByRole("button", { name: /^Remove/ }).elements()).toHaveLength(0);
    await stop.getByRole("button", { name: "Dismiss" }).click();

    await expect.poll(() => items().elements().length).toBe(0);
    expect(calls(adapter, "needs_you.dismiss")).toEqual([{ item: item.id }]);
    client.close();
  },
);

test(
  "a person denies the domain with a reason, which wall.denied_domains records",
  { tags: ["story:12-27", "screen:/needs-you", "operation:needs_you.deny_wall_domain"] },
  async () => {
    const adapter = consoleAdapter();
    const item = refetchStopped("user_configuration", CONFIGURATION);
    adapter.answer("needs_you.list", { items: [item] });
    adapter.answer("needs_you.deny_wall_domain", ({ domain, reason }) => {
      resolve(adapter, item.id);
      return { domain, added: true, reason };
    });
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    const reason = "The site's owner objected in writing.";
    const stop = items().nth(0);
    const field = stop.getByRole("textbox", { name: "Reason for the denial" });
    await expect.element(field).toBeRequired();
    await userEvent.type(field, reason);
    await stop.getByRole("button", { name: "Deny walled.test" }).click();

    await expect.poll(() => items().elements().length).toBe(0);
    expect(calls(adapter, "needs_you.deny_wall_domain")).toEqual([
      { domain: "walled.test", reason },
    ]);
    client.close();
  },
);

const PINNED = "sha256/pinned-key-of-the-test-host";
const PRESENTED = "sha256/changed-key-of-the-test-host";

const CHANGED: NeedsYouItem = {
  id: "certificate_changed:self-signed.test",
  kind: "certificate_changed",
  since: "2026-10-24T09:30:00Z",
  run: "nightly",
  summary: "self-signed.test presented another key.",
  actions: ["certs.accept", "certs.forget"],
  details: {
    host: "self-signed.test",
    person: "os_user",
    pinned_spki_sha256: PINNED,
    presented_spki_sha256: PRESENTED,
  },
};

test(
  "a certificate whose key changed shows both keys, and its person accepts the new key",
  { tags: ["story:12-28", "screen:/needs-you", "operation:certs.accept"] },
  async () => {
    const adapter = consoleAdapter();
    adapter.answer("needs_you.list", { items: [CHANGED] });
    adapter.answer("certs.accept", ({ host, spki_sha256 }) => {
      resolve(adapter, CHANGED.id);
      return { host, spki_sha256 };
    });
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    const changed = items().nth(0);
    await expect
      .element(changed.getByRole("heading"))
      .toHaveTextContent("The certificate of self-signed.test changed its key");
    await expect.element(changed).toMatchTextContent(PINNED);
    await expect.element(changed).toMatchTextContent(PRESENTED);
    expect(changed.getByRole("button", { name: "Dismiss" }).elements()).toHaveLength(0);
    await changed.getByRole("button", { name: "Accept the new key" }).click();

    await expect.poll(() => items().elements().length).toBe(0);
    expect(calls(adapter, "certs.accept")).toEqual([
      { host: "self-signed.test", spki_sha256: PRESENTED },
    ]);
    client.close();
  },
);

test(
  "a person forgets the pin of a host whose key changed",
  { tags: ["story:12-28", "screen:/needs-you", "operation:certs.forget"] },
  async () => {
    const adapter = consoleAdapter();
    adapter.answer("needs_you.list", { items: [CHANGED] });
    adapter.answer("certs.forget", ({ host }) => {
      resolve(adapter, CHANGED.id);
      return { host, forgotten: true };
    });
    const client = clientOf(adapter);
    await shown(client);
    await subscribed(adapter);

    await items().nth(0).getByRole("button", { name: "Forget the pin" }).click();

    await expect.poll(() => items().elements().length).toBe(0);
    expect(calls(adapter, "certs.forget")).toEqual([{ host: "self-signed.test" }]);
    client.close();
  },
);

test(
  "the render budget of Needs you: commits for an item added and for one resolved",
  { tags: ["screen:/needs-you"] },
  async () => {
    const adapter = consoleAdapter();
    adapter.answer("needs_you.list", { items: [LOW] });
    const client = clientOf(adapter);
    let commits = 0;
    await render(
      <ConsoleI18n>
        <Profiler id="needs-you" onRender={() => (commits += 1)}>
          <Shell client={client} route="/needs-you" />
        </Profiler>
      </ConsoleI18n>,
    );
    await expect.element(items().nth(0)).toMatchTextContent(FOLDER);
    await subscribed(adapter);
    /**
     * The commits `action` takes until `settled` holds and two frames more,
     * so a commit the action schedules after it shows is counted too.
     */
    const committed = async (action: () => void, settled: () => Promise<void>) => {
      commits = 0;
      action();
      await settled();
      await new Promise((frame) => requestAnimationFrame(() => requestAnimationFrame(frame)));
      return commits;
    };

    const added = await committed(
      () => {
        adapter.push({ event: "needs_you.changed", data: { change: "listed", item: STOPPED } }, 1);
      },
      async () => {
        await expect.element(menuLink()).toHaveTextContent("Needs you2");
        await expect.poll(() => items().elements().length).toBe(2);
      },
    );
    const resolved = await committed(
      () => {
        adapter.push(
          {
            event: "needs_you.changed",
            data: { change: "removed", id: LOW.id, reason: "resolved" },
          },
          2,
        );
      },
      async () => {
        await expect.element(menuLink()).toHaveTextContent("Needs you1");
        await expect.poll(() => items().elements().length).toBe(1);
      },
    );

    expect({ added, resolved }).toMatchSnapshot();
    client.close();
  },
);
