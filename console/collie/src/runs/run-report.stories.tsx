// Each state of a run's Run report (TESTING.md, Components): a report with
// every part, a bare one whose parts read as absent, a run stopped by
// memory, one a Limit stopped, one that failed with a code the catalog does
// not know, and a replayed run.
import type { RunReport } from "@goodfetch/collie-client/generated";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { RunReportView } from "./run-report.tsx";

const BARE: RunReport = {
  run_id: "nightly",
  lifecycle: "finished",
  verdict: "verified",
  completeness: "complete",
  started_at: "2026-10-24T08:00:00Z",
  ended_at: "2026-10-24T09:10:00Z",
  usage: { browser_time: 120_000, model: { calls: 3, tokens: 1200 }, proxy_bytes: 0, requests: 7 },
};

const EVERY_PART: RunReport = {
  ...BARE,
  lifecycle: "stopped",
  verdict: "needs_review",
  completeness: "incomplete",
  verdict_reasons: [{ code: "uncertain_effects", message: "An Effect is uncertain.", count: 1 }],
  completeness_reasons: [{ code: "stopped", message: "It stopped early: storage ran out." }],
  stop_reason: { kind: "resources", resource: "storage", folder: "/data/runs/nightly" },
  usage: {
    ...BARE.usage,
    limits: [
      {
        scope: { kind: "install", id: "" },
        usage_kind: "requests",
        amount: 1000,
        left: 993,
        set_by: "person",
      },
    ],
    model_allowance: [
      {
        kind: "price",
        currency: "USD",
        value: 500_000,
        remains: 0,
        reached_at: "2026-10-24T09:05:00Z",
        source: {
          key: "ai.allowance.price",
          value: { amount: "0.50", currency: "USD" },
          layer: "project_configuration",
        },
      },
    ],
  },
  plan: {
    file: "plans/shop.plan",
    version: 3,
    status: "pending_review",
    steps_changed: ["checkout"],
  },
  warnings: [
    { code: "cpu_throttled", message: "The host throttled the CPU." },
    { code: "path_not_writable", message: "The Plan file could not be written.", folder: "plans" },
  ],
  uncertain_effects: [
    {
      entry: "e1",
      key: "order-1",
      reason: "failed_after_send",
      first_command: "page.click('#pay')",
      since: "2026-10-24T09:02:00Z",
    },
  ],
  weakened: [
    {
      key: "tls.accept_invalid_certs",
      value: { "intranet.example": null },
      layer: "run",
      origin: "settings",
    },
  ],
  policy_version: "sha256:0227dc879fda140d",
  settings: [
    {
      key: "tls.accept_invalid_certs",
      products: ["collie", "foxhound"],
      applied: true,
      value: { "intranet.example": null },
      layer: "user_configuration",
      weakens: false,
    },
  ],
  plugins: [{ name: "sink-s3", network_level: "egress", confinement_missing: ["seccomp"] }],
  checkpoint: {
    run_id: "nightly",
    run_epoch: 1,
    location: "runs/nightly/checkpoints/1",
    digest: "sha256:abc",
    journal_position: 9,
    size: 512,
    time: "2026-10-24T09:00:00Z",
  },
  nothing_done: { reason: "nothing_pending" },
  recording: {
    mode: "replay",
    file: "tests/shop.warc",
    responses_used: 4,
    recorded_from: "2026-09-01T10:00:00Z",
    recorded_to: "2026-09-02T10:00:00Z",
    misses: ["https://shop.example/cart"],
  },
  error: { code: "path_not_writable", message: "The Plan file could not be written." },
};

/** A Limit's stop, with the fields the generated type does not name. */
const LIMIT_STOP = {
  kind: "limit",
  scope: { kind: "install", id: "" },
  usage_kind: "requests",
} as const;

const meta: Meta<typeof RunReportView> = {
  title: "Run report",
  component: RunReportView,
};

export default meta;

type Story = StoryObj<typeof RunReportView>;

/** A report with every part an area supplies. */
export const EveryPart: Story = { args: { report: EVERY_PART } };

/** A report with only the parts every run has: the others read as absent. */
export const Bare: Story = { args: { report: BARE } };

/** A run the engine ran out of memory for. */
export const OutOfMemory: Story = {
  args: {
    report: {
      ...BARE,
      lifecycle: "stopped",
      completeness: "incomplete",
      stop_reason: { kind: "resources", resource: "memory" },
    },
  },
};

/** A run a Limit stopped. */
export const StoppedByALimit: Story = {
  args: {
    report: {
      ...BARE,
      lifecycle: "stopped",
      completeness: "incomplete",
      stop_reason: LIMIT_STOP,
    },
  },
};

/** A run that failed with a code the catalog does not know: the engine's English message. */
export const FailedWithAnUnknownCode: Story = {
  args: {
    report: {
      ...BARE,
      lifecycle: "failed",
      verdict: "rejected",
      completeness: "incomplete",
      error: { code: "some_newer_code", message: "A newer engine's failure, in English." },
    },
  },
};
