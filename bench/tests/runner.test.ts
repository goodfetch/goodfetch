// Seam 2 (spec #152): the `bench` command line on fixed campaign files,
// against the commit's Proving ground on a fixed seed and the stub
// competitor adapter, whose correctness and speed the campaigns script.
// Carried over from the lab's `engine` tests: a failed conformance gate
// blocks measurement, a hung adapter times out alone, the probes measure the
// adapter's process tree or stay null, and the oracle is timed apart.
import { readdir } from "node:fs/promises";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import type { Attempt } from "../src/cell.ts";
import { bench, written } from "./bench.ts";

function statuses(attempts: readonly Attempt[], cell: string): string[] {
  return attempts
    .filter((attempt) => attempt.cell_id === cell)
    .toSorted((left, right) => left.index - right.index)
    .map((attempt) => `${attempt.phase}:${attempt.status}`);
}

describe("bench run", { tags: ["story:2-7"] }, () => {
  test.override("provingGroundSeed", 152);

  test("runs each adapter's cells in seeded blocks, judged by the oracle, with provenance", async ({
    provingGround,
  }) => {
    const ran = await bench("stubs.json", provingGround.address);
    expect(ran.stderr).toBe("");
    const { record, attempts } = await written(ran.out);

    expect({
      status: record.status,
      seed: record.proving_ground.seed,
      tasks: record.proving_ground.tasks.map((task) => task.id),
      granted: record.host.class_granted,
      orders: record.blocks.map((block) => block.order.toSorted()),
    }).toEqual({
      status: "complete",
      seed: 152,
      tasks: ["catalog/2"],
      granted: "local-dev",
      orders: [
        ["correct", "crawl-only", "wrong"],
        ["correct", "crawl-only", "wrong"],
      ],
    });
    expect(record.provenance.source_fingerprint).toMatch(/^[0-9a-f]{64}$/);
    expect(record.provenance.git?.sha ?? "no commit").toMatch(/^([0-9a-f]{40}|no commit)$/);

    // The correct stub is verified throughout; the wrong one's false success
    // fails the conformance gate, and an unclaimed kind is unsupported: the
    // plan stays visible as skipped attempts.
    const measured = ["warmup:skipped_gate", ...Array(3).fill("measure:skipped_gate")];
    for (const block of [0, 1]) {
      expect(statuses(attempts, `correct:catalog/2:b${block}`)).toEqual([
        "conformance:verified",
        "warmup:verified",
        ...Array(3).fill("measure:verified"),
      ]);
      expect(statuses(attempts, `wrong:catalog/2:b${block}`)).toEqual([
        "conformance:false_success",
        ...measured,
      ]);
      expect(statuses(attempts, `crawl-only:catalog/2:b${block}`)).toEqual([
        "conformance:unsupported",
        ...measured,
      ]);
    }
    expect(record.totals).toMatchObject({ verified: 10, false_success: 2, unsupported: 2 });

    // The oracle's time is its own figure, never part of the task's.
    for (const attempt of attempts.filter((one) => one.judgement !== null)) {
      expect(attempt.timing.oracle_ms).toBe(
        (attempt.judgement?.oracle_time_microseconds ?? Number.NaN) / 1000,
      );
      expect(attempt.timing.task_ms).toEqual(expect.any(Number));
    }

    // A metric the run cannot measure is null with its reason, never 0.
    for (const cell of record.cells) {
      for (const [id, measure] of Object.entries(cell.metrics)) {
        const shape = measure.value === null ? `null: ${measure.reason !== ""}` : "value";
        const availability = record.metric_availability[id];
        expect(availability?.available === false ? shape : "").toBe(
          availability?.available === false ? "null: true" : "",
        );
        expect(shape).not.toBe("null: false");
      }
    }
    const correct = record.cells.find((cell) => cell.cell_id === "correct:catalog/2:b0");
    const wrong = record.cells.find((cell) => cell.cell_id === "wrong:catalog/2:b0");
    expect({
      failureRate: correct?.metrics.failure_rate.value,
      rss: (correct?.metrics.rss_peak_bytes.value ?? 0) > 0,
      cdp: correct?.metrics.cdp_commands_per_task,
      throughput: wrong?.metrics.verified_tasks_per_second,
    }).toEqual({
      failureRate: 0,
      rss: true,
      cdp: { value: null, unit: "command/task", reason: "the adapter reports no CDP commands" },
      throughput: {
        value: null,
        unit: "task/s",
        reason: "the cell failed its conformance gate",
      },
    });
  });

  test("times out a hung adapter's stuck attempt alone, and measures on its restart", async ({
    provingGround,
  }) => {
    const ran = await bench("hang.json", provingGround.address);
    expect(ran.stderr).toBe("");
    const { record, attempts } = await written(ran.out);
    const measured = attempts.filter((attempt) => attempt.phase === "measure");

    expect({
      timeout: measured.filter((one) => one.status === "timeout").map((one) => one.error_code),
      aborted: measured.filter((one) => one.status === "aborted").map((one) => one.error_code),
      verified: measured.filter((one) => one.status === "verified").map((one) => one.generation),
      failureRate: record.cells[0]?.metrics.failure_rate.value,
    }).toEqual({
      timeout: ["adapter_timeout"],
      aborted: ["sibling_abort", "sibling_abort", "sibling_abort"],
      verified: [1, 1, 1, 1],
      // One timeout over it and the four verified: the aborted siblings count for neither.
      failureRate: 0.2,
    });
  });

  test("refuses a campaign task the Proving ground does not list", async ({ provingGround }) => {
    const ran = await bench("unknown-task.json", provingGround.address);

    expect({ code: ran.code, error: ran.stderr.split(":")[0] }).toEqual({
      code: 1,
      error: "error[task_unknown]",
    });
    expect(await readdir(ran.out)).toEqual([]);
  });
});
