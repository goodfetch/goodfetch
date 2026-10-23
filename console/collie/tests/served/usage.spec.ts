// Usage and Limits served by Collie's executable (spec #592; ADR 0041): for
// Limits seeded in the test's Usage ledger on every scope a Client may set
// one on (the install, the Daemon, the person, a named token and the
// Client connection), with a period in a time zone and a price Limit, the
// screen shows what `goodfetch-collie usage list --json` prints for the
// same ledger: each Limit by scope, what it allows, the Usage charged and
// held against it, what is left and who set it.
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import type { LimitUsage, UsageListResult } from "@goodfetch/collie-client/generated";
import { expect, test, type ServedDaemon } from "@goodfetch/test-support/playwright";

const run = promisify(execFile);

/** Runs `goodfetch-collie usage <args>` in the test's home, answering what it prints. */
async function usage(daemon: ServedDaemon, args: readonly string[]): Promise<string> {
  const { stdout } = await run(daemon.executable, ["usage", ...args], { env: { ...daemon.env } });
  return stdout;
}

/** Adds a Limit of the CLI's own on `scope`, with `flags`. */
async function set(daemon: ServedDaemon, scope: string, flags: readonly string[]): Promise<void> {
  await usage(daemon, ["set", "--scope", scope, ...flags]);
}

/** Each scope kind's name on the screen. */
const SCOPES: Readonly<Record<string, string>> = {
  install: "Install",
  daemon: "Daemon",
  person: "Person",
  token: "Named token",
  connection: "Client connection",
};

/** Who set a Limit, on the screen. */
const SETTERS: Readonly<Record<LimitUsage["limit"]["set_by"], string>> = {
  person: "A person",
  managed_policy: "Managed Operator policy",
  client: "A Client",
};

/** One Limit as the screen shows it: its scope, Usage kind, amounts in machine form and setter. */
/** `rows` in one order, whatever order they came in. */
function sorted(rows: readonly Shown[]): Shown[] {
  return rows.toSorted((left, right) => left.join("\t").localeCompare(right.join("\t")));
}

type Shown = readonly [
  scope: string,
  usageKind: string,
  amount: string,
  charged: string,
  held: string,
  left: string,
  setBy: string,
];

test(
  "Usage shows each Limit of the ledger by scope as usage list shows it",
  { annotation: [{ type: "covers", description: "story:12-99" }] },
  async ({ page, daemon, signIn }) => {
    await set(daemon, "install", [
      "--usage-kind",
      "requests",
      "--amount",
      "10000",
      "--period",
      '{"every":"month","time_zone":"Europe/Paris"}',
    ]);
    await set(daemon, "daemon", ["--usage-kind", "proxy_bytes", "--amount", "1000000"]);
    await set(daemon, "person", ["--usage-kind", "model_calls", "--amount", "200"]);
    await set(daemon, "token", [
      "--scope-id",
      "nightly-ci",
      "--usage-kind",
      "browser_time",
      "--amount",
      "3600000",
      "--period",
      '{"every":"day","time_zone":"Asia/Kathmandu"}',
    ]);
    await set(daemon, "connection", [
      "--usage-kind",
      "paid_solves",
      "--currency",
      "EUR",
      "--amount",
      "5000000",
    ]);
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- `usage list --json` prints `goodfetch-protocol`'s type, which the executable's own tests check against the bundle.
    const listed = JSON.parse(await usage(daemon, ["list", "--json"])) as UsageListResult;
    expect(listed.limits.map(({ limit }) => limit.scope.kind).toSorted()).toEqual([
      "connection",
      "daemon",
      "install",
      "person",
      "token",
    ]);

    await signIn.withLink(page);
    await page.goto(new URL("/settings", daemon.url).href);
    const table = page.getByRole("table", { name: "Usage against each Limit" });
    await expect(table).toBeVisible();

    const shown: Shown[] = [];
    for (const row of await table
      .getByRole("row")
      .filter({ has: page.getByRole("rowheader") })
      .all()) {
      const cells = row.getByRole("cell");
      const value = async (index: number): Promise<string> =>
        (await cells.nth(index).locator("data").getAttribute("value")) ?? "";
      shown.push([
        (await row.getByRole("rowheader").textContent()) ?? "",
        (await cells.nth(0).locator("code").textContent()) ?? "",
        await value(2),
        await value(3),
        await value(4),
        await value(5),
        (await cells.nth(6).textContent()) ?? "",
      ]);
    }
    const expected = listed.limits.map(({ limit, charged, held, left }): Shown => [
      `${SCOPES[limit.scope.kind] ?? limit.scope.kind}${limit.scope.id === "" ? "" : ` ${limit.scope.id}`}`,
      limit.usage_kind,
      String(limit.amount),
      String(charged),
      String(held),
      String(left),
      SETTERS[limit.set_by],
    ]);
    expect(sorted(shown)).toEqual(sorted(expected));
    await expect(table.getByRole("rowheader")).toHaveText([
      "Install",
      /^Daemon /,
      /^Person /,
      "Named token nightly-ci",
      /^Client connection /,
    ]);
  },
);
