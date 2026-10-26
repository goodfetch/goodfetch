// A run's page per Lifecycle and Verdict (ADR 0073's `visual` screens),
// against the in-memory adapter of the protocol, in the pinned Playwright
// image only (TESTING.md, Visual regression). The runs carry no point in
// time the page shows, so no clock needs fixing.
import type {
  RunIndexRow,
  RunProgress,
  RunProgressParams,
} from "@goodfetch/collie-client/generated";
import { createProtocolAdapter, failure } from "@goodfetch/test-support/protocol-adapter";
import { expect, test } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";
import { createConsoleClient, type PageVisibility } from "#client/client.ts";
import { ConsoleI18n } from "#i18n.tsx";
import { Shell } from "#shell.tsx";
import "#styles.css";

/** A page that stays visible. */
const visible: PageVisibility = { visible: true, watch: () => () => {} };

const RUN_ID = "nightly";

test.each([
  { name: "running", axes: { lifecycle: "running" } },
  {
    name: "finished-verified",
    axes: { lifecycle: "finished", verdict: "verified", completeness: "complete" },
  },
  {
    name: "finished-needs-review",
    axes: { lifecycle: "finished", verdict: "needs_review", completeness: "complete" },
  },
  {
    name: "stopped-verified",
    axes: { lifecycle: "stopped", verdict: "verified", completeness: "incomplete" },
  },
  {
    name: "stopped-needs-review",
    axes: { lifecycle: "stopped", verdict: "needs_review", completeness: "incomplete" },
  },
  {
    name: "failed-rejected",
    axes: { lifecycle: "failed", verdict: "rejected", completeness: "unverified" },
  },
] satisfies { name: string; axes: Pick<RunIndexRow, "lifecycle" | "verdict" | "completeness"> }[])(
  "a $name run's page",
  { tags: ["story:12-11", "screen:/runs/$run"] },
  async ({ name, axes }) => {
    const live = axes.lifecycle === "running";
    const progress: RunProgress = {
      run_id: RUN_ID,
      position: 1,
      at: "2026-10-24T09:00:00Z",
      lifecycle: axes.lifecycle,
      writer: live ? "live" : "none",
      epoch: 1,
      pauses: [],
      message: live ? "Read 12 of 40 pages" : null,
      last: !live,
    };
    const adapter = createProtocolAdapter();
    adapter.answer("needs_you.list", { items: [] });
    adapter.answer("run.progress", (params: RunProgressParams) =>
      params.after === undefined || params.after === null ? progress : new Promise(() => {}),
    );
    adapter.answer("run.usage", {
      browser_time: 0,
      model: { calls: 0, tokens: 0 },
      proxy_bytes: 0,
      requests: 0,
    });
    // The Run report is kept off these References until they are written again
    // with it (no pinned image here); its own tests show it.
    adapter.answer("run.report", failure("run_state_unavailable", { run_id: RUN_ID }));
    adapter.answer("run.list", {
      runs: [
        {
          run_id: RUN_ID,
          location: `runs/${RUN_ID}`,
          kind: "collie",
          needs: "collie",
          tags: [],
          started_at: "2026-10-24T08:00:00Z",
          presence: "present",
          replayed: false,
          // A live run's index row names the engine that owns it (ADR 0031).
          owner: live ? { daemon: { tcp: "127.0.0.1:7801" } } : null,
          ...axes,
        },
      ],
    });
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
    await expect.element(page.getByRole("term").filter({ hasText: "Lifecycle" })).toBeVisible();

    await expect.element(page.getByRole("main")).toMatchScreenshot(`run-page-${name}`);
    client.close();
  },
);
