// Each state of Usage and Limits (TESTING.md, Components): reading the
// ledger, its Limits on every scope (a managed install Limit, a Daemon's, a
// person's reached, a named token's, a Client connection's price Limit, a
// Live view time Limit understood, a row only Foxhound carries and one a
// newer engine wrote), no Limit at all, the engine's refusal; and a run's
// Usage, its replayed responses apart from the requests it fetched.
import { RunStateUnavailableError, type LimitUsage } from "@goodfetch/collie-client/generated";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { RunUsageView } from "./run-usage.tsx";
import { UsageView } from "./usage-section.tsx";

/** A Limit on the install's requests, as `usage.list` answers it, overridden per row. */
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

const everyScope: readonly LimitUsage[] = [
  limit({
    limit: {
      amount: 10_000,
      period: { every: "month", time_zone: "Europe/Paris" },
      set_by: "managed_policy",
    },
    charged: 4_000,
    held: 100,
    left: 5_900,
    resets_at: "2026-10-31T23:00:00Z",
    managed_major: "collie-0",
  }),
  limit({
    limit: {
      usage_kind: "live_view",
      amount: 7_200,
      period: { every: "day", time_zone: "Asia/Kathmandu" },
      set_by: "person",
    },
    charged: 600,
    left: 6_600,
    resets_at: "2026-10-23T18:15:00Z",
  }),
  limit({
    limit: {
      usage_kind: "paid_fetch",
      currency: "USD",
      amount: 20_000_000,
      must_understand: false,
    },
    standing: "foxhound",
    left: 20_000_000,
  }),
  limit({ limit: { usage_kind: "gpu_seconds" }, standing: "not_understood" }),
  limit({
    limit: {
      scope: { kind: "daemon", id: "collie-0" },
      usage_kind: "proxy_bytes",
      amount: 1_000_000,
      period: { every: "week", time_zone: "America/New_York" },
    },
    left: 1_000_000,
    resets_at: "2026-10-26T04:00:00Z",
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
      scope: { kind: "token", id: "nightly-ci" },
      usage_kind: "browser_time",
      amount: 3_600_000,
      period: { every: "day", time_zone: "UTC" },
    },
    left: 3_600_000,
    resets_at: "2026-10-24T00:00:00Z",
  }),
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
];

const meta: Meta<typeof UsageView> = {
  title: "Engine & settings/Usage and Limits",
  component: UsageView,
};

export default meta;

type Story = StoryObj<typeof UsageView>;

/** Waiting for `usage.list`'s answer. */
export const Reading: Story = { args: { read: { kind: "reading" } } };

/** A Limit on every scope, with every setter and standing. */
export const EveryScope: Story = {
  args: { read: { kind: "read", usage: { limits: [...everyScope] } } },
};

/** No Limit holds the user's runs. */
export const NoLimit: Story = { args: { read: { kind: "read", usage: { limits: [] } } } };

/** The engine could not read the ledger. */
export const Failed: Story = {
  args: {
    read: {
      kind: "failed",
      error: new RunStateUnavailableError(
        "the Usage ledger cannot be read or written: database disk image is malformed",
        { details: { operation: "usage.list", path: "/srv/goodfetch/1/usage-ledger.sqlite" } },
      ),
    },
  },
};

/** A run's Usage: requests fetched, responses replayed apart, and the Limits that apply to it. */
export const RunUsage: StoryObj<typeof RunUsageView> = {
  render: (args) => <RunUsageView {...args} />,
  args: {
    usage: {
      requests: 12,
      browser_time: 84_000,
      proxy_bytes: 0,
      model: { calls: 3, tokens: 1_200 },
      paid_solves: [{ currency: "EUR", amount: 20_000 }],
      limits: [
        {
          scope: { kind: "install", id: "" },
          usage_kind: "requests",
          amount: 10_000,
          left: 5_900,
          period: { every: "month", time_zone: "Europe/Paris" },
          resets_at: "2026-10-31T23:00:00Z",
          set_by: "managed_policy",
        },
      ],
    },
    recording: { mode: "replay", file: "tests/login.warc", responses_used: 30 },
  },
};
