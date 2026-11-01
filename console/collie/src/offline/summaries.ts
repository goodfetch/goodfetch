// What the offline cache keeps of a run (ADR 0031: secret fields are never
// cached). Each summary is built field by field from an allow-list, so a
// field the engine adds later is never kept by accident. Left out: the text
// a program writes, its status line, its own tags and its Human pauses'
// reasons, where a secret the engine's Mask does not know could sit; and
// where the engine that owns a live run is reached, with its token's file.
// Goodfetch's own tags (`goodfetch.<key>:<value>`, ADR 0036), which only
// its starters set, are kept.
import type { RunIndexRow, RunProgress } from "@goodfetch/collie-client/generated";
import type { ShownRun } from "#runs/run-page.tsx";

/** What begins each tag only Goodfetch's own starters set (ADR 0036). */
const RESERVED = "goodfetch.";

/** What the cache keeps of a run's Run index row. */
export function cachedRow(row: RunIndexRow): RunIndexRow {
  return {
    run_id: row.run_id,
    location: row.location,
    kind: row.kind,
    needs: row.needs,
    tags: row.tags.filter((tag) => tag.startsWith(RESERVED)),
    lifecycle: row.lifecycle,
    verdict: row.verdict ?? null,
    completeness: row.completeness ?? null,
    started_at: row.started_at,
    ended_at: row.ended_at ?? null,
    owner: null,
    presence: row.presence,
    progress_message: null,
    stop_reason: row.stop_reason ?? null,
    replayed: row.replayed,
  };
}

/** What the cache keeps of a run's progress. */
function cachedProgress(progress: RunProgress): RunProgress {
  return {
    run_id: progress.run_id,
    position: progress.position,
    at: progress.at,
    lifecycle: progress.lifecycle,
    writer: progress.writer,
    epoch: progress.epoch,
    checkpoint_at: progress.checkpoint_at ?? null,
    pauses: [],
    message: null,
    storage:
      progress.storage === undefined || progress.storage === null
        ? null
        : {
            folder: progress.storage.folder,
            free_bytes: progress.storage.free_bytes,
            level: progress.storage.level,
          },
    stop_reason: progress.stop_reason ?? null,
    last: progress.last,
  };
}

/** What the cache keeps of what a run's page shows. */
export function cachedRun(run: ShownRun): ShownRun {
  return {
    progress: cachedProgress(run.progress),
    axes: {
      lifecycle: run.axes.lifecycle,
      ...(run.axes.verdict === undefined ? {} : { verdict: run.axes.verdict }),
      ...(run.axes.completeness === undefined ? {} : { completeness: run.axes.completeness }),
    },
    needs: run.needs,
    uncapped: run.uncapped,
  };
}
