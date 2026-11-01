// Usage and Limits in Engine & settings against the in-memory adapter of
// the protocol (spec #592, seam 1; ADRs 0041 and 0105): the screen shows
// each Limit `usage.list` answers, by scope (install, Daemon, person, named
// token, Client connection), with what it allows, the Usage charged and
// held against it, what is left, who set it and whether this engine
// understands a row flagged *must understand*. Amounts and times are
// asserted in their machine form, never formatted (TESTING.md,
// Localization).
import type { LimitUsage, UsageListResult } from "@goodfetch/collie-client/generated";
import { createProtocolAdapter, failure } from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
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

/** A Limit on the install's requests, as `usage.list` answers it, overridden where a test asserts. */
function limit(
  overrides: Partial<Omit<LimitUsage, "limit">> & {
    readonly limit?: Partial<LimitUsage["limit"]>;
  } = {},
): LimitUsage {
  const { limit: row = {}, ...usage } = overrides;
  return {
    limit: {
      scope: { kind: "install", id: "" },
      usage_kind: "requests",
      amount: 1000,
      must_understand: true,
      set_by: "client",
      ...row,
    },
    standing: "held",
    charged: 0,
    held: 0,
    left: 1000,
    ...usage,
  };
}

/** Engine & settings, signed in, over the adapter, whose `usage.list` answers `answer`. */
async function usageOf(answer: UsageListResult | ReturnType<typeof failure>) {
  // The adapter over every operation the Console calls, the person-only `config.policy_read` included.
  const adapter = createProtocolAdapter<ConsoleOperations>();
  adapter.answer("needs_you.list", { items: [] });
  adapter.answer("config.explain", { files: [], settings: [] });
  adapter.answer("usage.list", answer);
  adapter.answer("config.policy_read", {
    path: "/home/1/operator-policy.toml",
    found: false,
    digest: "sha256:e3b0c44298fc1c14",
    entries: [],
  });
  const client = createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "collie",
  });
  await render(
    <ConsoleI18n>
      <Shell client={client} route="/settings" />
    </ConsoleI18n>,
  );
  return { adapter, client };
}

/** The Usage table. */
function limits() {
  return page.getByRole("table", { name: "Usage against each Limit" });
}

/** What the row of the Limit on `usageKind` shows: each cell's text, and the machine value of each amount and time in it. */
async function rowOf(usageKind: string): Promise<{
  readonly scope: string;
  readonly cells: readonly string[];
  readonly values: readonly (string | null)[];
}> {
  const row = limits().getByRole("row").filter({ hasText: usageKind });
  await expect.element(row).toBeVisible();
  const scope = row.getByRole("rowheader").element().textContent ?? "";
  const cells = row.getByRole("cell").elements();
  return {
    scope,
    cells: cells.map((cell) => cell.textContent ?? ""),
    values: cells.map(
      (cell) =>
        cell.querySelector("data")?.getAttribute("value") ??
        cell.querySelector("time")?.getAttribute("datetime") ??
        null,
    ),
  };
}

test(
  "Usage shows each Limit by scope with what it allows, what is charged and held, what is left and who set it",
  { tags: ["story:12-99", "screen:/settings"] },
  async () => {
    const { client } = await usageOf({
      limits: [
        limit({
          limit: {
            scope: { kind: "connection", id: "9f3a-1" },
            usage_kind: "paid_solves",
            currency: "EUR",
            amount: 5_000_000,
          },
          charged: 1_250_000,
          left: 3_750_000,
        }),
        limit({
          limit: {
            usage_kind: "requests",
            amount: 10_000,
            period: { every: "month", time_zone: "Europe/Paris" },
            set_by: "managed_policy",
          },
          charged: 4_000,
          held: 100,
          left: 5_900,
          period_started_at: "2026-09-30T22:00:00Z",
          resets_at: "2026-10-31T23:00:00Z",
          managed_major: "collie-0",
        }),
        limit({
          limit: {
            scope: { kind: "token", id: "nightly-ci" },
            usage_kind: "browser_time",
            amount: 3_600_000,
            period: { every: "day", time_zone: "UTC" },
          },
          resets_at: "2026-10-24T00:00:00Z",
          left: 3_600_000,
        }),
        limit({
          limit: {
            scope: { kind: "person", id: "jb" },
            usage_kind: "model_calls",
            amount: 200,
            set_by: "person",
          },
          charged: 200,
          left: 0,
        }),
        limit({
          limit: {
            scope: { kind: "daemon", id: "collie-0" },
            usage_kind: "proxy_bytes",
            amount: 1_000_000,
            period: { every: "week", time_zone: "America/New_York" },
          },
          resets_at: "2026-10-26T04:00:00Z",
          left: 1_000_000,
        }),
      ],
    });

    await expect.element(limits()).toBeVisible();
    const scopes = limits()
      .getByRole("rowheader")
      .elements()
      .map((cell) => cell.textContent);
    expect(scopes).toEqual([
      "Install",
      "Daemon collie-0",
      "Person jb",
      "Named token nightly-ci",
      "Client connection 9f3a-1",
    ]);

    const install = await rowOf("requests");
    expect(install.cells[0]).toBe("requests");
    expect(install.cells[1]).toMatch(/^Each month in Europe\/Paris, starting again /);
    expect(install.values.slice(1, 6)).toEqual([
      "2026-10-31T23:00:00Z",
      "10000",
      "4000",
      "100",
      "5900",
    ]);
    expect(install.cells.slice(6)).toEqual(["Managed Operator policy", "Yes, understood"]);

    const connection = await rowOf("paid_solves");
    expect(connection.cells[0]).toBe("paid_solves in EUR");
    expect(connection.cells[1]).toBe("No period");
    expect(connection.values.slice(2, 6)).toEqual(["5000000", "1250000", "0", "3750000"]);
    expect(connection.cells.slice(6)).toEqual(["A Client", "Yes, understood"]);

    const person = await rowOf("model_calls");
    expect(person.values.slice(2, 6)).toEqual(["200", "200", "0", "0"]);
    expect(person.cells[6]).toBe("A person");
    client.close();
  },
);

test(
  "a Limit on Live view time flagged must understand shows as understood by this engine",
  { tags: ["story:12-99"] },
  async () => {
    const { client } = await usageOf({
      limits: [
        limit({
          limit: {
            usage_kind: "live_view",
            amount: 7_200,
            period: { every: "day", time_zone: "Asia/Kathmandu" },
            must_understand: true,
            set_by: "person",
          },
          resets_at: "2026-10-23T18:15:00Z",
          charged: 600,
          left: 6_600,
        }),
      ],
    });

    const row = await rowOf("live_view");
    expect(row.cells.slice(6)).toEqual(["A person", "Yes, understood"]);
    client.close();
  },
);

test("a row this engine cannot evaluate, and one only Foxhound carries, say so", async () => {
  const { client } = await usageOf({
    limits: [
      limit({ limit: { usage_kind: "gpu_seconds" }, standing: "not_understood" }),
      limit({
        limit: { usage_kind: "paid_fetch", currency: "USD", must_understand: false },
        standing: "foxhound",
      }),
    ],
  });

  expect((await rowOf("gpu_seconds")).cells.at(-1)).toBe(
    "Yes, not understood by this engine: runs it may cover are refused",
  );
  expect((await rowOf("paid_fetch")).cells.at(-1)).toBe("No, Foxhound's, kept as written");
  client.close();
});

test("with no Limit in the ledger, Usage says nothing holds the runs", async () => {
  const { client } = await usageOf({ limits: [] });

  await expect
    .element(
      page.getByRole("region", { name: "Usage and Limits" }).getByText("No Limit holds your runs."),
    )
    .toBeVisible();
  expect(limits().elements()).toEqual([]);
  client.close();
});

test(
  "a ledger the engine cannot read shows its refusal in the catalog's words, its fields as data",
  { tags: ["story:12-133"] },
  async () => {
    const { client } = await usageOf(
      failure("run_state_unavailable", {
        operation: "usage.list",
        fix: "free some space on the disk of the Goodfetch home",
        path: "/home/jb/.goodfetch/usage-ledger.sqlite",
      }),
    );
    const alert = page.getByRole("region", { name: "Usage and Limits" }).getByRole("alert");

    await expect
      .element(alert)
      .toHaveTextContent(
        "The engine could not read the Usage ledger:" +
          "usage.list: The run's state is missing, damaged or cannot be written." +
          "Fixfree some space on the disk of the Goodfetch home" +
          "Error coderun_state_unavailable",
      );
    expect(
      [
        "usage.list",
        "free some space on the disk of the Goodfetch home",
        "run_state_unavailable",
      ].map((value) =>
        alert.getByText(value, { exact: true }).element().closest('[translate="no"]'),
      ),
    ).not.toContain(null);
    client.close();
  },
);
