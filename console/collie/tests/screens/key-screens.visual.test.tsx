// The key screens ADR 0073 lists for the `visual` projects, beside the run's
// page per Lifecycle and Verdict (`run-page.visual.test.tsx`) and an error
// state (`error-state.visual.test.tsx`): the page that is not signed in,
// Runs, "Needs you" with each item kind, and Engine & settings, against the
// in-memory adapter of the protocol, in the pinned Playwright image only
// (TESTING.md, Visual regression). The points in time they show are
// literal, formatted under the visual project's fixed locale and time zone,
// and nothing on them reads the clock.
import type {
  ExplainedSetting,
  NeedsYouItem,
  RunIndexRow,
} from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import type { Address } from "#address.ts";
import {
  createConsoleClient,
  type ConsoleClient,
  type ConsoleOperations,
  type PageVisibility,
} from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";
import "#styles.css";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

/** An address with no query. */
const address: Address = { search: "", replace: () => {} };

/** An engine with nothing in "Needs you", or `items`. */
function engine(items: NeedsYouItem[] = []): ProtocolAdapter<ConsoleOperations> {
  const adapter = createProtocolAdapter<ConsoleOperations>();
  adapter.answer("needs_you.list", { items });
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

async function shellAt(client: ConsoleClient | undefined, route: string): Promise<void> {
  await render(
    <ConsoleI18n>
      <Shell client={client} route={route} address={address} />
    </ConsoleI18n>,
  );
}

test("the Console's page when it is not signed in", async () => {
  await shellAt(undefined, "/");
  await expect.element(page.getByRole("status")).toBeVisible();

  await expect.element(page.getByRole("main")).toMatchScreenshot("signed-out");
});

function row(run_id: string, overrides: Partial<RunIndexRow>): RunIndexRow {
  return {
    run_id,
    location: `runs/${run_id}`,
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

test("Runs, a live run beside ended ones", async () => {
  const adapter = engine();
  adapter.answer("run.list", {
    runs: [
      row("nightly", {
        tags: ["team:shop"],
        progress_message: "Read 12 of 40 pages",
        owner: { daemon: { tcp: "127.0.0.1:7801" } },
      }),
      row("prices", {
        lifecycle: "finished",
        verdict: "verified",
        completeness: "complete",
        started_at: "2026-10-23T08:00:00Z",
        ended_at: "2026-10-23T09:00:00Z",
        tags: ["goodfetch.recipe:shop-prices"],
      }),
      row("catalog", {
        lifecycle: "stopped",
        verdict: "needs_review",
        completeness: "incomplete",
        started_at: "2026-10-22T08:00:00Z",
        ended_at: "2026-10-22T08:30:00Z",
      }),
    ],
  });
  const client = clientOf(adapter);
  await shellAt(client, "/runs");
  await expect.element(page.getByRole("rowheader").filter({ hasText: "catalog" })).toBeVisible();

  await expect.element(page.getByRole("main")).toMatchScreenshot("runs");
  client.close();
});

const FOLDER = "/home/ada/.goodfetch";

test.each([
  {
    id: `storage_level:${FOLDER}`,
    kind: "storage_level",
    since: "2026-10-24T09:00:00Z",
    summary: `The filesystem holding ${FOLDER} is at the low storage level.`,
    actions: [],
    details: { level: "low", folder: FOLDER },
  },
  {
    id: "run_stopped_by_storage:nightly:2026-10-24T09:05:00Z",
    kind: "run_stopped_by_storage",
    since: "2026-10-24T09:05:00Z",
    run: "nightly",
    summary: "The Run nightly stopped: the filesystem holding its folder filled.",
    actions: ["needs_you.dismiss"],
    details: { folder: `${FOLDER}/1/runs/nightly` },
  },
  {
    id: "run_stopped_by_limit:capped:2026-10-24T09:10:00Z",
    kind: "run_stopped_by_limit",
    since: "2026-10-24T09:10:00Z",
    run: "capped",
    summary: "The Run capped stopped: a Limit on requests has nothing left.",
    actions: ["usage.raise", "needs_you.dismiss"],
    details: {
      limit: {
        scope: { kind: "install", id: "" },
        usage_kind: "requests",
        period: { every: "month", time_zone: "Europe/Paris" },
        resets_at: "2026-10-31T23:00:00Z",
      },
      amount: 1000,
      needs: "collie",
    },
  },
  {
    id: "consent_wall:walled.example",
    kind: "consent_wall",
    since: "2026-10-24T09:10:00Z",
    run: "nightly",
    summary: "walled.example showed a consent wall.",
    actions: ["needs_you.allow_consent_domain", "needs_you.dismiss"],
    details: { domain: "walled.example" },
  },
  {
    id: "wall_refetch_stopped:shop.example",
    kind: "wall_refetch_stopped",
    since: "2026-10-24T09:30:00Z",
    run: "nightly",
    summary: "shop.example refused a page fetched again without its stored cookies and storage.",
    actions: ["needs_you.remove_refetch_domain", "needs_you.deny_wall_domain", "needs_you.dismiss"],
    details: {
      domain: "shop.example",
      cause: "challenge",
      layer: "user_configuration",
      file: "/home/ada/.config/goodfetch/config.toml",
      only_star: false,
    },
  },
  {
    id: "certificate_changed:intranet.example",
    kind: "certificate_changed",
    since: "2026-10-24T09:40:00Z",
    run: "nightly",
    summary: "intranet.example presented another key.",
    actions: ["certs.accept", "certs.forget"],
    details: {
      host: "intranet.example",
      person: "os_user",
      pinned_spki_sha256: "sha256/pinned-key-of-the-example-host",
      presented_spki_sha256: "sha256/changed-key-of-the-example-host",
    },
  },
] satisfies NeedsYouItem[])('"Needs you" with a $kind item', async (item) => {
  const client = clientOf(engine([item]));
  await shellAt(client, "/needs-you");
  await expect.element(page.getByRole("list", { name: "Needs you" })).toBeVisible();

  await expect.element(page.getByRole("main")).toMatchScreenshot(`needs-you-${item.kind}`);
  client.close();
});

/** A setting as `config.explain` answers it. */
function setting(key: string, overrides: Partial<ExplainedSetting>): ExplainedSetting {
  return {
    key,
    products: ["collie", "foxhound"],
    applied: true,
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a setting's value is any JSON value, which the generated type writes as an object.
    value: 30000 as unknown as ExplainedSetting["value"],
    layer: "default",
    weakens: false,
    user_settable: true,
    fixed_for_run: false,
    ...overrides,
  };
}

test("Engine & settings", async () => {
  const adapter = engine();
  adapter.answer("config.explain", {
    files: [],
    settings: [
      setting("timeout.navigation", {}),
      setting("timeout.run", {
        // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- as above.
        value: 600000 as unknown as ExplainedSetting["value"],
        layer: "user_configuration",
      }),
    ],
  });
  adapter.answer("usage.list", { limits: [] });
  adapter.answer("config.policy_read", {
    path: "/home/ada/.goodfetch/operator-policy.toml",
    found: false,
    digest: "sha256:e3b0c44298fc1c14",
    entries: [],
  });
  const client = clientOf(adapter);
  await shellAt(client, "/settings");
  await expect.element(page.getByRole("grid", { name: "Effective settings" })).toBeVisible();

  await expect.element(page.getByRole("main")).toMatchScreenshot("engine-settings");
  client.close();
});
