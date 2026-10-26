// What the offline cache keeps of a run (ADR 0031: secret fields are never
// cached): a run's Run index row and its page, each without the text a
// program writes, where a secret the engine's Mask does not know could
// sit, and without where the engine that owns a live run is reached.
import type { RunIndexRow, RunProgress } from "@goodfetch/collie-client/generated";
import { describe, expect, test } from "vitest";
import type { ShownRun } from "#runs/run-page.tsx";
import { cachedRow, cachedRun } from "./summaries.ts";

const ROW: RunIndexRow = {
  run_id: "shop-crawl-17",
  location: "runs/shop-crawl-17",
  kind: "collie",
  needs: "collie",
  tags: ["goodfetch.recipe:shop", "team:password-hunter2"],
  lifecycle: "stopped",
  verdict: null,
  completeness: null,
  started_at: "2026-10-25T09:00:00Z",
  ended_at: "2026-10-25T09:05:00Z",
  owner: { daemon: { unix: "/run/goodfetch/daemon.sock" } },
  presence: "present",
  replayed: false,
  progress_message: "logged in with hunter2",
  stop_reason: { kind: "pause" },
};

const PROGRESS: RunProgress = {
  run_id: "shop-crawl-17",
  position: 4,
  at: "2026-10-25T09:05:00Z",
  lifecycle: "stopped",
  writer: "none",
  epoch: 2,
  checkpoint_at: "2026-10-25T09:04:00Z",
  pauses: [
    {
      page: "page-1",
      session: "session-1",
      since: "2026-10-25T09:03:00Z",
      raised_by: { by: "program" },
      reason: "type the code hunter2",
    },
  ],
  message: "logged in with hunter2",
  storage: { folder: "/data", free_bytes: 10, level: "low" },
  stop_reason: { kind: "pause" },
  last: true,
};

describe("what the offline cache keeps of a run", () => {
  test("a Run index row keeps its axes, times and Goodfetch's own tags, never a program's text or its owner", () => {
    expect(cachedRow(ROW)).toEqual({
      ...ROW,
      tags: ["goodfetch.recipe:shop"],
      owner: null,
      progress_message: null,
    });
  });

  test("a run's page keeps its axes, writer, Checkpoint and storage, never its status line or Human pauses", () => {
    const shown: ShownRun = {
      progress: PROGRESS,
      axes: { lifecycle: "stopped" },
      needs: "collie",
      uncapped: false,
    };

    expect(JSON.stringify(cachedRun(shown))).not.toContain("hunter2");
    expect(cachedRun(shown)).toEqual({
      ...shown,
      progress: { ...PROGRESS, pauses: [], message: null },
    });
  });
});
