// A run's Run report on its page against the in-memory adapter of the
// protocol (spec #592, seam 1; story 15; ADRs 0029, 0041, 0102, 0105, 0106,
// 0107, 0116 and 0122): once the run ended, its page shows the report the
// engine answered `run.report` with, each part in a section of its own: the
// axes with their reasons, the stop reason (a `resources` stop naming
// memory or storage and its folder), Usage against the Limits, the Model
// allowance apart from them with what remains, when it was reached and the
// setting and layer that set it, the Plan report, warnings with their
// codes, uncertain Effects, the settings that weakened a safeguard, the
// policy version, settings with their layers, Plugins, the last
// Checkpoint, start and end, why nothing was done, the error its ending
// raised through the message catalog, and a replayed Response recording
// with its mark, mode, file, responses used, fetch times and misses. A
// part the report does not carry reads as absent, never as zero.
import type {
  RunIndexRow,
  RunProgress,
  RunProgressParams,
  RunReport,
  RunUsage,
} from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page, type Locator } from "vitest/browser";
import { render } from "vitest-browser-react";
import { createConsoleClient, type ConsoleClient, type PageVisibility } from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

const RUN_ID = "nightly";

const USAGE: RunUsage = {
  browser_time: 0,
  model: { calls: 3, tokens: 1200 },
  proxy_bytes: 0,
  requests: 7,
};

/** The fewest parts a report carries: the axes, start and end, and Usage. */
const BARE: RunReport = {
  run_id: RUN_ID,
  lifecycle: "finished",
  verdict: "verified",
  completeness: "complete",
  started_at: "2026-10-24T08:00:00Z",
  ended_at: "2026-10-24T09:10:00Z",
  usage: USAGE,
};

/** A report with every part an area supplies. */
const SEEDED: RunReport = {
  ...BARE,
  lifecycle: "stopped",
  verdict: "needs_review",
  completeness: "incomplete",
  verdict_reasons: [{ code: "uncertain_effects", message: "An Effect is uncertain.", count: 1 }],
  completeness_reasons: [{ code: "stopped", message: "It stopped early: storage ran out." }],
  stop_reason: { kind: "resources", resource: "storage", folder: "/data/runs/nightly" },
  usage: {
    ...USAGE,
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
    {
      code: "path_not_writable",
      message: "The Plan file could not be written.",
      folder: "plans",
    },
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
    run_id: RUN_ID,
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
  error: {
    code: "path_not_writable",
    message: "The Plan file `plans/shop.plan` could not be written.",
  },
};

function progressOf(lifecycle: RunProgress["lifecycle"]): RunProgress {
  return {
    run_id: RUN_ID,
    position: 1,
    at: "2026-10-24T09:10:00Z",
    lifecycle,
    writer: lifecycle === "running" ? "live" : "none",
    epoch: 1,
    pauses: [],
    last: lifecycle !== "running",
  };
}

function rowOf(report: RunReport): RunIndexRow {
  return {
    run_id: RUN_ID,
    location: `runs/${RUN_ID}`,
    kind: "collie",
    needs: "collie",
    tags: [],
    lifecycle: report.lifecycle,
    verdict: report.verdict,
    completeness: report.completeness,
    started_at: report.started_at,
    ended_at: report.ended_at,
    presence: "present",
    replayed: report.recording?.mode === "replay",
  };
}

/** The engine of an ended run whose report is `report`. */
function engineOf(report: RunReport): ProtocolAdapter {
  const adapter = createProtocolAdapter();
  adapter.answer("needs_you.list", { items: [] });
  adapter.answer("run.progress", (params: RunProgressParams) =>
    params.after === undefined || params.after === null
      ? progressOf(report.lifecycle)
      : new Promise(() => {}),
  );
  adapter.answer("run.usage", report.usage);
  adapter.answer("run.list", { runs: [rowOf(report)] });
  adapter.answer("run.report", report);
  return adapter;
}

async function openReport(report: RunReport): Promise<ConsoleClient> {
  const adapter = engineOf(report);
  const client = createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "collie",
  });
  await render(
    <ConsoleI18n>
      <Shell client={client} route={`/runs/${RUN_ID}`} />
    </ConsoleI18n>,
  );
  await expect
    .poll(() => page.getByRole("heading", { name: "Run report" }).elements())
    .toHaveLength(1);
  return client;
}

/** The text of every element `locator` finds. */
function textOf(locator: Locator): string {
  return locator
    .elements()
    .map((element) => element.textContent)
    .join("");
}

/** The attribute `name` of the first element `locator` finds. */
function attributeOf(locator: Locator, name: string): string | null | undefined {
  return locator.elements()[0]?.getAttribute(name);
}

/** The report's section headed `name`. */
function section(name: string) {
  return page.getByRole("region", { name, exact: true });
}

test(
  "an ended run's page shows each part of its Run report",
  { tags: ["story:12-15", "screen:/runs/$run", "operation:run.report"] },
  async () => {
    const client = await openReport(SEEDED);

    const ending = section("How it ended");
    await expect.poll(() => textOf(ending)).toMatch(/Ended asstopped/);
    await expect.poll(() => textOf(ending)).toMatch(/Its Verdictneeds review/);
    await expect
      .poll(() => textOf(page.getByRole("list", { name: "Why it is not verified" })))
      .toMatch("uncertain_effects: An Effect is uncertain. (1)");
    await expect
      .poll(() => textOf(page.getByRole("list", { name: "Why it is not complete" })))
      .toMatch("stopped: It stopped early: storage ran out.");
    await expect
      .poll(() => textOf(ending))
      .toMatch(
        "The engine ran out of storage for it: the filesystem of /data/runs/nightly filled.",
      );
    await expect
      .poll(() => textOf(ending))
      .toMatch("Nothing was done: a resume or a retry found nothing left.");
    await expect.poll(() => textOf(ending)).toMatch(/Started.+Ended.+Checkpoint/);
    // The ending's error reads in the catalog's words, its code beside it.
    await expect.poll(() => textOf(ending)).toMatch("The path cannot be written.");
    await expect.poll(() => textOf(ending)).toMatch("Error codepath_not_writable");

    await expect
      .element(
        section("Usage against its Limits").getByRole("table", {
          name: "Limits that apply to this run",
        }),
      )
      .toBeVisible();

    const allowance = section("Model allowance");
    await expect.poll(() => textOf(allowance)).toMatch("A Model allowance is not a Limit");
    await expect.poll(() => textOf(allowance)).toMatch(/KindPrice/);
    await expect.poll(() => textOf(allowance)).toMatch(/Allows\$0\.50.*Remains\$0\.00/);
    await expect
      .poll(() => textOf(allowance))
      .toMatch(
        'ai.allowance.price = {"amount":"0.50","currency":"USD"}, at the Project configuration layer',
      );
    await expect
      .poll(() => attributeOf(allowance.getByRole("time"), "datetime"))
      .toBe("2026-10-24T09:05:00Z");
    // The allowance is not among the Limits.
    await expect
      .poll(() => textOf(section("Usage against its Limits")))
      .not.toMatch("ai.allowance");

    const recording = section("Response recording");
    await expect.poll(() => textOf(recording)).toMatch("Replayed");
    await expect.poll(() => textOf(recording)).toMatch(/Modereplay/);
    await expect.poll(() => textOf(recording)).toMatch(/Recording filetests\/shop.warc/);
    await expect.poll(() => textOf(recording)).toMatch(/Responses used4/);
    await expect.poll(() => textOf(recording)).toMatch(/Misses1/);
    await expect
      .poll(() => textOf(page.getByRole("list", { name: "Requests the recording missed" })))
      .toMatch("https://shop.example/cart");
    await expect
      .poll(() => attributeOf(recording.getByRole("time").first(), "datetime"))
      .toBe("2026-09-01T10:00:00Z");

    const plan = section("Plan report");
    await expect.poll(() => textOf(plan)).toMatch(/Plan fileplans\/shop.plan/);
    await expect
      .poll(() => textOf(plan))
      .toMatch("pending review: its new versions wait for a person");
    await expect.poll(() => textOf(plan)).toMatch("checkout");

    const warnings = page.getByRole("list", { name: "Warnings" });
    // A code the catalog does not know keeps the engine's English message; one it knows reads in its words.
    await expect.poll(() => textOf(warnings)).toMatch("cpu_throttled: The host throttled the CPU.");
    await expect
      .poll(() => textOf(warnings))
      .toMatch("path_not_writable: The path cannot be written.");
    await expect.poll(() => textOf(warnings)).toMatch("Folder: plans");

    await expect
      .poll(() => textOf(page.getByRole("list", { name: "Uncertain Effects" })))
      .toMatch(
        /The Effect order-1 is uncertain \(failed_after_send\) since .+First command sent: page\.click\('#pay'\)/,
      );

    await expect
      .poll(() => textOf(page.getByRole("list", { name: "Settings that weakened a safeguard" })))
      .toMatch(
        'tls.accept_invalid_certs = {"intranet.example":null}, set by Run, from the run\'s settings',
      );

    const configuration = section("Settings and Plugins");
    await expect.poll(() => textOf(configuration)).toMatch(/Policy versionsha256:0227dc879fda140d/);
    await expect
      .poll(() => textOf(configuration.getByRole("grid", { name: "Effective settings" })))
      .toMatch(/tls\.accept_invalid_certs\{"intranet\.example":null\}User configuration/);
    await expect
      .poll(() => textOf(page.getByRole("list", { name: "Plugins" })))
      .toMatch("sink-s3 at the network level egress without seccomp");
    client.close();
  },
);

test(
  "a part the Run report does not carry is shown as absent, never as zero",
  { tags: ["story:12-15", "screen:/runs/$run"] },
  async () => {
    const client = await openReport(BARE);

    await expect
      .poll(() => textOf(section("How it ended")))
      .toMatch(/Stop reasonit did not stop early/);
    await expect.poll(() => textOf(section("How it ended"))).toMatch(/Checkpointnone written/);
    await expect
      .poll(() => textOf(section("Model allowance")))
      .toMatch("No Model allowance is set for this run.");
    await expect
      .poll(() => textOf(section("Response recording")))
      .toMatch("The run named no Response recording: it fetched live.");
    await expect
      .poll(() => textOf(section("Plan report")))
      .toMatch("The report has no Plan part: the run used no Plan file.");
    await expect.poll(() => textOf(section("Warnings"))).toMatch("The run warns of nothing.");
    await expect
      .poll(() => textOf(section("Uncertain Effects")))
      .toMatch("The report has no uncertain Effects part.");
    await expect
      .poll(() => textOf(section("Weakened safeguards")))
      .toMatch("No setting weakened a safeguard for this run.");
    const configuration = section("Settings and Plugins");
    await expect.poll(() => textOf(configuration)).toMatch("no Operator policy applied");
    await expect.poll(() => textOf(configuration)).toMatch("The report has no settings part.");
    await expect.poll(() => textOf(configuration)).toMatch("The report has no Plugins part.");
    expect(page.getByText("Replayed", { exact: true }).elements()).toHaveLength(0);
    client.close();
  },
);

test.each([
  {
    name: "memory",
    stop: { kind: "resources", resource: "memory" } as const,
    says: "The engine ran out of memory for it.",
  },
  {
    name: "storage, naming its folder",
    stop: { kind: "resources", resource: "storage", folder: "/mnt/small" } as const,
    says: "The engine ran out of storage for it: the filesystem of /mnt/small filled.",
  },
  {
    name: "a Limit",
    stop: {
      kind: "limit",
      scope: { kind: "install", id: "" },
      usage_kind: "proxy_bytes",
      currency: "USD",
    } as const,
    says: "A Limit on install capping proxy_bytes (USD) was reached.",
  },
])(
  "a run stopped by $name says so in its report",
  { tags: ["story:12-15", "screen:/runs/$run"] },
  async ({ stop, says }) => {
    const client = await openReport({ ...BARE, lifecycle: "stopped", stop_reason: stop });

    await expect.poll(() => textOf(section("How it ended"))).toMatch(says);
    client.close();
  },
);

test(
  "an ending error whose code the catalog does not know keeps the engine's English message",
  { tags: ["story:12-15", "screen:/runs/$run"] },
  async () => {
    const client = await openReport({
      ...BARE,
      lifecycle: "failed",
      error: { code: "some_newer_code", message: "A newer engine's failure, in English." },
    });

    await expect
      .poll(() => textOf(section("How it ended")))
      .toMatch("A newer engine's failure, in English.");
    client.close();
  },
);

test(
  "a replayed run is marked on its page above its report, so it never reads as live",
  { tags: ["story:12-15", "screen:/runs/$run"] },
  async () => {
    const client = await openReport(SEEDED);

    await expect
      .poll(() => textOf(page.getByRole("main")))
      .toMatch(
        /^.*Replayed This run replayed a Response recording: it is no live run\..*How it stands/s,
      );
    client.close();
  },
);
