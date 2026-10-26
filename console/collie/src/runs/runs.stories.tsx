// Each state of the Runs screen (TESTING.md, Components): reading, runs in
// every Lifecycle, Verdict and Completeness with a missing and an erased
// row, a Schedule's run and its Watch run shown, a filter that matches
// nothing, the engine's refusal, and the page with no Connection.
import {
  FormatVersionUnsupportedError,
  type RunIndexRow,
} from "@goodfetch/collie-client/generated";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { RunsView } from "./runs.tsx";

function row(run_id: string, overrides: Partial<RunIndexRow> = {}): RunIndexRow {
  return {
    run_id,
    location: `runs/${run_id}`,
    kind: "collie",
    needs: "collie",
    tags: [],
    lifecycle: "running",
    started_at: "2026-10-01T08:00:00Z",
    presence: "present",
    replayed: false,
    ...overrides,
  };
}

const everyState: RunIndexRow[] = [
  row("01kb7x3y4z5a6b7c8d9e0f1g2h", {
    tags: ["team:shop"],
    started_at: "2026-10-05T08:00:00Z",
    progress_message: "Read 12 of 40 pages",
    owner: { daemon: { unix: "/home/ada/.local/share/goodfetch/0.1/daemon.sock" } },
  }),
  row("nightly.20261004t080000z.target.abcd2345", {
    tags: [
      "goodfetch.schedule:nightly",
      "goodfetch.occurrence:nightly.20261004t080000z",
      "goodfetch.trigger:time",
      "goodfetch.role:target",
      "goodfetch.recipe:shop-prices",
    ],
    lifecycle: "finished",
    verdict: "verified",
    completeness: "complete",
    started_at: "2026-10-04T08:00:00Z",
    ended_at: "2026-10-04T08:20:00Z",
  }),
  row("nightly.20261004t080000z.watch.wxyz6789", {
    tags: [
      "goodfetch.schedule:nightly",
      "goodfetch.occurrence:nightly.20261004t080000z",
      "goodfetch.trigger:time",
      "goodfetch.role:watch",
    ],
    lifecycle: "finished",
    verdict: "verified",
    completeness: "complete",
    started_at: "2026-10-04T07:59:00Z",
    ended_at: "2026-10-04T07:59:40Z",
  }),
  row("catalog-crawl", {
    kind: "crawl",
    needs: "foxhound",
    tags: ["goodfetch.recipe:catalog-crawl"],
    lifecycle: "failed",
    verdict: "needs_review",
    completeness: "unverified",
    started_at: "2026-10-03T08:00:00Z",
    ended_at: "2026-10-03T09:00:00Z",
  }),
  row("price-check", {
    lifecycle: "stopped",
    verdict: "rejected",
    completeness: "incomplete",
    started_at: "2026-10-02T08:00:00Z",
    ended_at: "2026-10-02T08:05:00Z",
    presence: "missing",
  }),
  row("old-export", {
    lifecycle: "finished",
    verdict: "verified",
    completeness: "complete",
    started_at: "2026-10-01T08:00:00Z",
    ended_at: "2026-10-01T08:01:00Z",
    presence: "erased",
  }),
];

const meta: Meta<typeof RunsView> = {
  title: "Runs",
  component: RunsView,
  args: { filter: {}, onFilter: (): void => {} },
};

export default meta;

type Story = StoryObj<typeof RunsView>;

/** Waiting for `run.list`'s answer. */
export const Reading: Story = { args: { listed: { kind: "reading" } } };

/** Runs in every state, a missing and an erased row among them. */
export const EveryState: Story = {
  args: { listed: { kind: "listed", runs: everyState }, filter: { watch_runs: true } },
};

/** A filter in the address that names fields of the "More filters" group, which opens. */
export const Filtered: Story = {
  args: {
    listed: { kind: "listed", runs: everyState.slice(1, 2) },
    filter: {
      schedule: "nightly",
      trigger: "time",
      started: { from: "2026-10-04T00:00:00Z" },
      sort: "ended_at",
      order: "ascending",
    },
  },
};

/** No run matches the filter. */
export const NoMatch: Story = {
  args: { listed: { kind: "listed", runs: [] }, filter: { lifecycle: "failed" } },
};

/** The engine refused to list the runs. */
export const Failed: Story = {
  args: {
    listed: {
      kind: "failed",
      error: new FormatVersionUnsupportedError(
        "The Run index is run-index 2.0, which this engine cannot read",
        { fix: "upgrade Collie", details: { operation: "run.list", format: "run-index 2.0" } },
      ),
    },
  },
};

/** The Console has no Connection: the rows shown stay, and filtering is disabled. */
export const ReadOnly: Story = {
  args: { listed: { kind: "listed", runs: everyState }, readOnly: true },
};
