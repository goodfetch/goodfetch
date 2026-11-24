// The Linux probes' readers, carried over from the lab's `linux-probes`
// tests: a process's CPU from `/proc/<pid>/stat` and its proportional memory
// from `/proc/<pid>/smaps_rollup`.
import { expect, test } from "vitest";
import { detectProbes } from "./metrics.ts";
import { parseProcStat, parseSmapsRollup, sampleProcessTree } from "./resources.ts";

function measured(value: number | null): string {
  return value === null ? "null" : value > 0 ? "positive" : "not positive";
}

test("measures a process tree with the probes the host has, and leaves the rest null", async () => {
  const probes = detectProbes(process.platform);
  const usage = await sampleProcessTree({ pid: process.pid, probes, intervalMs: 50 }).stop();
  const expected = (probe: "proc_stat" | "smaps_rollup" | "ps"): string =>
    probes.has(probe) ? "positive" : "null";
  expect({
    cpuMs: measured(usage.cpuMs),
    peakPssBytes: measured(usage.peakPssBytes),
    peakRssBytes: measured(usage.peakRssBytes),
  }).toEqual({
    cpuMs: expected("proc_stat"),
    peakPssBytes: expected("smaps_rollup"),
    peakRssBytes: expected("ps"),
  });

  const none = await sampleProcessTree({
    pid: process.pid,
    probes: new Set(),
    intervalMs: 50,
  }).stop();
  expect(none).toEqual({ cpuMs: null, peakPssBytes: null, peakRssBytes: null });
});

test("reads a process's CPU after its parenthesized command name", () => {
  // The command name may hold spaces and parentheses; utime and stime are
  // fields 14 and 15, in clock ticks.
  const stat =
    "4242 (node (worker) x) S 1 4242 4242 0 -1 4194304 1200 0 0 0 250 75 0 0 20 0 11 0 9000 1234567 3000 18446744073709551615";
  expect(parseProcStat(stat, 100)).toEqual({
    pid: 4242,
    ppid: 1,
    userMs: 2500,
    systemMs: 750,
  });
});

test("reads proportional anonymous and swap memory in bytes", () => {
  const rollup = [
    "55d0f0a00000-7ffd5f9fe000 ---p 00000000 00:00 0 [rollup]",
    "Rss:              102400 kB",
    "Pss:               51200 kB",
    "Pss_Anon:          40960 kB",
    "Pss_File:          10240 kB",
    "Pss_Shmem:             0 kB",
    "SwapPss:            1024 kB",
  ].join("\n");
  expect(parseSmapsRollup(rollup)).toEqual({
    pssBytes: 51200 * 1024,
    pssAnonBytes: 40960 * 1024,
    swapPssBytes: 1024 * 1024,
  });
});
