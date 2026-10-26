// Each state of a run's page (TESTING.md, Components): reading, a Run ID
// the Run index does not list, the engine's refusal, a live run with Human
// pauses waiting, a low storage level and the uncapped mark, a run in each
// Lifecycle and Verdict, an interrupted run with its resume command, the
// notices of its actions, the page with no Connection, a live run with its
// owning engine, and a run that needs Foxhound on Collie's engine, each
// Foxhound view refused with `not_in_product`.
import {
  NotInProductError,
  RunStateUnavailableError,
  type RunProgress,
} from "@goodfetch/collie-client/generated";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { FOXHOUND_VIEWS } from "./foxhound-views.ts";
import { RunPageView, type RunAxisValues, type ShownRun } from "./run-page.tsx";

const RUN_ID = "01kb7x3y4z5a6b7c8d9e0f1g2h";

function progressOf(overrides: Partial<RunProgress> = {}): RunProgress {
  return {
    run_id: RUN_ID,
    position: 4,
    at: "2026-10-24T09:00:00Z",
    lifecycle: "running",
    writer: "live",
    epoch: 1,
    pauses: [],
    last: false,
    ...overrides,
  };
}

/** A run that ended with `axes`, no engine working it any more. */
function ended(axes: RunAxisValues): ShownRun {
  return {
    progress: progressOf({
      lifecycle: axes.lifecycle,
      writer: "none",
      last: true,
      checkpoint_at: "2026-10-24T08:45:00Z",
    }),
    axes,
    needs: "collie",
    uncapped: false,
  };
}

const LIVE: ShownRun = {
  progress: progressOf({
    epoch: 2,
    checkpoint_at: "2026-10-24T08:45:00Z",
    message: "Read 12 of 40 pages",
    storage: { level: "low", folder: "/srv/goodfetch/1/runs/nightly", free_bytes: 52_428_800 },
    pauses: [
      {
        session: "shop",
        page: "page-1",
        reason: "Log in with the MFA code sent to the account's phone",
        raised_by: { by: "program" },
        since: "2026-10-24T08:50:00Z",
      },
      {
        session: "shop",
        page: "page-2",
        raised_by: { by: "engine", step: "challenge_ladder" },
        since: "2026-10-24T08:55:00Z",
        deadline: "2026-10-24T09:55:00Z",
      },
    ],
  }),
  axes: { lifecycle: "running" },
  needs: "collie",
  uncapped: true,
};

const INTERRUPTED: ShownRun = {
  progress: progressOf({ writer: "none", epoch: 2, last: true }),
  axes: { lifecycle: "running" },
  needs: "collie",
  uncapped: false,
};

const meta: Meta<typeof RunPageView> = {
  title: "Run page",
  component: RunPageView,
  args: { runId: RUN_ID },
};

export default meta;

type Story = StoryObj<typeof RunPageView>;

/** Waiting for the run's first read. */
export const Reading: Story = { args: { read: { kind: "reading" } } };

/** The Run index lists no run with this Run ID. */
export const NotListed: Story = { args: { runId: "no-such-run", read: { kind: "not_listed" } } };

/** The engine refused to read the run. */
export const Failed: Story = {
  args: {
    read: {
      kind: "failed",
      error: new RunStateUnavailableError(
        "The state of the Run `nightly` is unavailable: run.sqlite is locked",
        {
          fix: "close the program that holds run.sqlite, then open the run again",
          details: {
            operation: "run.info",
            observed: "run.sqlite is locked",
            call_log: ["opening run.sqlite", "run.sqlite is locked"],
          },
        },
      ),
    },
  },
};

/** A live run: Human pauses waiting, a low storage level, uncapped, with stop and checkpoint. */
export const Live: Story = { args: { read: { kind: "shown", run: LIVE } } };

/** A live run whose Checkpoint was just written. */
export const CheckpointWritten: Story = {
  args: {
    read: { kind: "shown", run: LIVE },
    notice: { kind: "checkpoint_written", at: "2026-10-24T09:01:00Z" },
  },
};

/** Finished, verified and complete. */
export const FinishedVerified: Story = {
  args: {
    read: {
      kind: "shown",
      run: ended({ lifecycle: "finished", verdict: "verified", completeness: "complete" }),
    },
  },
};

/** Finished but needs review. */
export const FinishedNeedsReview: Story = {
  args: {
    read: {
      kind: "shown",
      run: ended({ lifecycle: "finished", verdict: "needs_review", completeness: "complete" }),
    },
  },
};

/** Stopped, verified but incomplete, with its resume command. */
export const StoppedVerifiedIncomplete: Story = {
  args: {
    read: {
      kind: "shown",
      run: ended({ lifecycle: "stopped", verdict: "verified", completeness: "incomplete" }),
    },
  },
};

/** Failed and rejected. */
export const FailedRejected: Story = {
  args: {
    read: {
      kind: "shown",
      run: ended({ lifecycle: "failed", verdict: "rejected", completeness: "unverified" }),
    },
  },
};

/** Interrupted: no engine works it, and its resume command was copied. */
export const Interrupted: Story = {
  args: { read: { kind: "shown", run: INTERRUPTED }, notice: { kind: "copied" } },
};

/** The Console has no Connection: the run shown stays, and stop and checkpoint are disabled. */
export const ReadOnly: Story = { args: { read: { kind: "shown", run: LIVE }, readOnly: true } };

/** A crawl that needs Foxhound, on Collie's engine: the views every run has, and each Foxhound view refused. */
export const NeedsFoxhound: Story = {
  args: {
    read: {
      kind: "shown",
      run: {
        ...ended({ lifecycle: "finished", verdict: "verified", completeness: "incomplete" }),
        needs: "foxhound",
      },
    },
    foxhoundViews: Object.fromEntries(
      FOXHOUND_VIEWS.map(({ operation }) => [
        operation,
        {
          kind: "refused",
          error: new NotInProductError(
            `\`${operation}\` is part of Foxhound, and this engine is Collie 0.1.0.`,
            {
              fix: "Run it with Foxhound, which contains everything Collie does (`npm install @goodfetch/foxhound`), or connect to a Foxhound engine.",
              details: { product: "collie", needs: "foxhound", requested: operation },
            },
          ),
        },
      ]),
    ),
  },
};

/** A live run owned by the Daemon, the Product it needs named. */
export const LiveOwned: Story = {
  args: {
    read: {
      kind: "shown",
      run: {
        ...LIVE,
        owner: { daemon: { unix: "/home/ada/.local/share/goodfetch/0.1/daemon.sock" } },
      },
    },
  },
};
