// The Console's first launch offers a monthly Limit for the install
// against the in-memory adapter of the protocol (spec #591, story 8; ADRs
// 0029 and 0041): with no install Limit in the Usage ledger, the Overview
// offers one; accepted, it is set through the person-only `usage.raise`, a
// month in the browser's time zone on paid spending in dollars; declined,
// the offer is not made again on the next visit; and it is not made while
// an install Limit holds.
import type { LimitUsage, UsageRaiseParams } from "@goodfetch/collie-client/generated";
import { createProtocolAdapter } from "@goodfetch/test-support/protocol-adapter";
import { beforeEach, expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import {
  createConsoleClient,
  type ConsoleOperations,
  type PageVisibility,
} from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

/** The browser's time zone, which the offered month counts in. */
const ZONE = Intl.DateTimeFormat().resolvedOptions().timeZone;

/** The install's monthly Limit `usage.raise` set to `params`, as the engine answers it. */
function setTo(params: UsageRaiseParams): LimitUsage {
  return {
    limit: {
      scope: { kind: "install", id: "" },
      usage_kind: params.usage_kind,
      currency: params.currency ?? null,
      period: params.period ?? null,
      amount: params.amount,
      must_understand: true,
      set_by: "person",
    },
    charged: 0,
    held: 0,
    left: params.amount,
    standing: "held",
  };
}

/** The Overview, signed in, over an adapter whose ledger holds `limits`. */
async function overview(limits: readonly LimitUsage[] = []) {
  const adapter = createProtocolAdapter<ConsoleOperations>();
  adapter.answer("needs_you.list", { items: [] });
  adapter.answer("usage.list", { limits: [...limits] });
  adapter.answer("usage.raise", (params) => setTo(params));
  const client = createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "collie",
  });
  const shown = await render(
    <ConsoleI18n>
      <Shell client={client} route="/" />
    </ConsoleI18n>,
  );
  return { adapter, client, shown };
}

/** The offer, a region of the Overview. */
function offer() {
  return page.getByRole("region", { name: "Cap this install's spending" });
}

beforeEach(() => {
  localStorage.clear();
});

test(
  "the first launch offers a monthly install Limit, which accepted is set through usage.raise",
  { tags: ["story:12-8", "operation:usage.raise"] },
  async () => {
    const { adapter, client } = await overview();
    await expect.element(offer()).toBeVisible();

    const field = page.getByRole("textbox", { name: "Each month, at most (USD)" });
    await userEvent.clear(field);
    await userEvent.type(field, "25");
    await userEvent.click(page.getByRole("button", { name: "Set the monthly Limit" }));

    const set = page.getByRole("status").filter({ hasText: "spending is capped" });
    await expect.element(set).toBeVisible();
    expect(set.element().querySelector("data")?.getAttribute("value")).toBe("25000000");
    expect(
      adapter.requests
        .filter((request) => request.method === "usage.raise")
        .map((request) => request.params),
    ).toEqual([
      {
        scope: "install",
        usage_kind: "paid_solves",
        currency: "USD",
        period: { every: "month", time_zone: ZONE },
        amount: 25_000_000,
      },
    ]);
    client.close();
  },
);

test("a declined offer is not made again on the next visit", { tags: ["story:12-8"] }, async () => {
  const first = await overview();
  await expect.element(offer()).toBeVisible();
  await userEvent.click(page.getByRole("button", { name: "Not now" }));
  await expect.element(offer()).not.toBeInTheDocument();
  first.client.close();
  await first.shown.unmount();

  const next = await overview();
  await expect.element(page.getByText("Connected to Collie", { exact: false })).toBeVisible();

  await expect.element(offer()).not.toBeInTheDocument();
  expect(localStorage.length).toBe(1);
  next.client.close();
});

test("no offer is made while an install Limit holds", { tags: ["story:12-8"] }, async () => {
  const { adapter, client } = await overview([
    setTo({ scope: "install", usage_kind: "requests", amount: 1000 }),
  ]);
  await expect
    .poll(() => adapter.requests.some((request) => request.method === "usage.list"))
    .toBe(true);

  await expect.element(offer()).not.toBeInTheDocument();
  client.close();
});
