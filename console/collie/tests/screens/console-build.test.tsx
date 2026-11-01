// The shell as a Product's build entry composes it (ADRs 0087, 0094 and
// 0098; spec #593), against the in-memory adapter of the protocol: a
// build names its Product in the page's title and first screen, and a
// build that carries views of its own shows them, each in a tab beside the
// views every run has, for a run that needs Foxhound, while a Collie run
// shows only the views every run has. The build here is a stand-in for
// Foxhound's, whose own views are tested in its package: Collie's never
// imports them.
import type { RunIndexRow, RunProgress, RunUsage } from "@goodfetch/collie-client/generated";
import {
  createProtocolAdapter,
  type ProtocolAdapter,
} from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page, userEvent } from "vitest/browser";
import { render } from "vitest-browser-react";
import type { ConsoleBuild } from "#console-build.ts";
import { createConsoleClient, type ConsoleClient, type PageVisibility } from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

/** A build of Foxhound's with one view of its own, which shows the Run ID it was given. */
const FOXHOUND_BUILD: ConsoleBuild = {
  product: "foxhound",
  runViews: [
    {
      id: "seeds",
      name: { id: "test.seeds", message: "Seeds" },
      content: ({ runId }) => <p>The seeds of {runId}</p>,
    },
  ],
};

/** A run's Usage, none spent. */
const USAGE: RunUsage = {
  browser_time: 0,
  model: { calls: 0, tokens: 0 },
  proxy_bytes: 0,
  requests: 0,
};

function progressOf(runId: string): RunProgress {
  return {
    run_id: runId,
    position: 1,
    at: "2026-10-24T09:00:00Z",
    lifecycle: "finished",
    writer: "none",
    epoch: 1,
    pauses: [],
    last: true,
  };
}

function rowOf(runId: string, needs: RunIndexRow["needs"]): RunIndexRow {
  return {
    run_id: runId,
    location: `runs/${runId}`,
    kind: needs === "foxhound" ? "crawl" : "collie",
    needs,
    tags: [],
    lifecycle: "finished",
    verdict: "verified",
    completeness: "complete",
    started_at: "2026-10-24T08:00:00Z",
    presence: "present",
    replayed: false,
  };
}

/** Foxhound's engine, whose Run index holds a crawl and a Collie run. */
function foxhoundEngine(): ProtocolAdapter {
  const adapter = createProtocolAdapter({ product: "foxhound" });
  const rows = [rowOf("crawl-run", "foxhound"), rowOf("collie-run", "collie")];
  adapter.answer("needs_you.list", { items: [] });
  adapter.answer("run.progress", (params) => progressOf(params.run_id));
  adapter.answer("run.usage", USAGE);
  adapter.answer("run.list", { runs: rows });
  adapter.answer("usage.list", { limits: [] });
  adapter.answer("run.report", (params) => ({
    run_id: params.run_id,
    lifecycle: "finished",
    verdict: "verified",
    completeness: "complete",
    started_at: "2026-10-24T08:00:00Z",
    ended_at: "2026-10-24T09:10:00Z",
    usage: USAGE,
  }));
  return adapter;
}

function clientOf(adapter: ProtocolAdapter): ConsoleClient {
  return createConsoleClient({
    transport: adapter.transport,
    bodyFetcher: adapter.bodyFetcher,
    visibility: visible,
    product: "foxhound",
  });
}

test(
  "Foxhound's build names Goodfetch Foxhound, then Foxhound, in the page's title and first screen",
  { tags: ["story:12-201"] },
  async () => {
    const client = clientOf(foxhoundEngine());

    await render(
      <ConsoleI18n>
        <Shell client={client} route="/" build={FOXHOUND_BUILD} />
      </ConsoleI18n>,
    );

    const main = page.getByRole("main");
    await expect
      .element(main.getByRole("heading", { level: 1 }))
      .toHaveTextContent("Goodfetch Foxhound");
    await expect.element(main.getByText("Foxhound's Console.", { exact: false })).toBeVisible();
    await expect
      .element(main.getByRole("status"))
      .toHaveTextContent("Connected to Foxhound 0.1.0.");
    expect(document.title).toBe("Goodfetch Foxhound");
    client.close();
  },
);

test(
  "in Foxhound's build, a run that needs Foxhound shows the build's views beside the views every run has",
  { tags: ["story:12-203", "screen:/runs/$run"] },
  async () => {
    const client = clientOf(foxhoundEngine());

    await render(
      <ConsoleI18n>
        <Shell client={client} route="/runs/crawl-run" build={FOXHOUND_BUILD} />
      </ConsoleI18n>,
    );

    const tabs = page.getByRole("tablist", { name: "Run views" }).getByRole("tab");
    await expect
      .poll(() => tabs.elements().map((tab) => tab.textContent))
      .toEqual(["Run", "Seeds"]);
    expect(document.title).toBe("Run · Goodfetch Foxhound");
    await userEvent.click(page.getByRole("tab", { name: "Seeds" }));
    await expect
      .element(page.getByRole("tabpanel", { name: "Seeds" }))
      .toHaveTextContent("The seeds of crawl-run");
    client.close();
  },
);

test(
  "in Foxhound's build, a Collie run shows only the views every run has",
  { tags: ["story:12-203", "screen:/runs/$run"] },
  async () => {
    const client = clientOf(foxhoundEngine());

    await render(
      <ConsoleI18n>
        <Shell client={client} route="/runs/collie-run" build={FOXHOUND_BUILD} />
      </ConsoleI18n>,
    );

    await expect.element(page.getByRole("region", { name: "How it stands" })).toBeVisible();
    await expect.element(page.getByRole("tablist")).not.toBeInTheDocument();
    client.close();
  },
);
