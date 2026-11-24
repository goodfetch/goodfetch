// What an adapter's process tree uses while a run measures it: CPU from
// `/proc/<pid>/stat` and proportional memory from `smaps_rollup` on Linux, and
// RSS from `ps` everywhere, sampled at an interval. A probe the host lacks
// gives `null`, never 0.
import { execFile } from "node:child_process";
import { readFile, readdir } from "node:fs/promises";
import { promisify } from "node:util";
import type { Probe } from "./metrics.ts";

const exec = promisify(execFile);

/** One process's line of `/proc/<pid>/stat`. */
export interface ProcStat {
  readonly pid: number;
  readonly ppid: number;
  readonly userMs: number;
  readonly systemMs: number;
}

/** Reads `/proc/<pid>/stat`, whose command name may hold spaces and parentheses. */
export function parseProcStat(text: string, clockTicksPerSecond: number): ProcStat {
  const close = text.lastIndexOf(")");
  // The fields after the command, from field 3 (state): ppid is field 4,
  // utime 14 and stime 15.
  const fields = text
    .slice(close + 2)
    .trim()
    .split(/\s+/);
  const ticks = (index: number): number => (Number(fields[index]) * 1000) / clockTicksPerSecond;
  return {
    pid: Number(text.slice(0, text.indexOf(" "))),
    ppid: Number(fields[1]),
    userMs: ticks(11),
    systemMs: ticks(12),
  };
}

/** A process's proportional memory, from `/proc/<pid>/smaps_rollup`. */
export interface SmapsRollup {
  readonly pssBytes: number;
  readonly pssAnonBytes: number;
  readonly swapPssBytes: number;
}

/** Reads `/proc/<pid>/smaps_rollup`; a field it lacks reads 0. */
export function parseSmapsRollup(text: string): SmapsRollup {
  const bytes = (name: string): number => {
    const match = new RegExp(`^${name}:\\s+(\\d+) kB`, "m").exec(text);
    return match?.[1] === undefined ? 0 : Number(match[1]) * 1024;
  };
  return {
    pssBytes: bytes("Pss"),
    pssAnonBytes: bytes("Pss_Anon"),
    swapPssBytes: bytes("SwapPss"),
  };
}

/** What a process tree used over a sampled window; `null` where no probe reads it. */
export interface TreeUsage {
  /** User plus system CPU, the highest sum sampled: a child's CPU leaves with it. */
  readonly cpuMs: number | null;
  /** Peak of the summed Pss_Anon plus SwapPss. */
  readonly peakPssBytes: number | null;
  /** Peak of the summed RSS, shared pages counted once per process. */
  readonly peakRssBytes: number | null;
}

async function maybeRead(path: string): Promise<string | null> {
  try {
    return await readFile(path, "utf8");
  } catch {
    // The process exited between the listing and the read.
    return null;
  }
}

function descendants(root: number, parents: ReadonlyMap<number, number>): Set<number> {
  const tree = new Set([root]);
  let changed = true;
  while (changed) {
    changed = false;
    for (const [pid, ppid] of parents) {
      if (tree.has(ppid) && !tree.has(pid)) {
        tree.add(pid);
        changed = true;
      }
    }
  }
  return tree;
}

async function procTree(root: number): Promise<{ cpuMs: number; pssBytes: number }> {
  const stats = new Map<number, ProcStat>();
  for (const entry of await readdir("/proc")) {
    if (/^\d+$/.test(entry)) {
      const text = await maybeRead(`/proc/${entry}/stat`);
      if (text !== null) {
        // `/proc` counts in USER_HZ, 100 on x86-64 and arm64.
        const stat = parseProcStat(text, 100);
        stats.set(stat.pid, stat);
      }
    }
  }
  const parents = new Map([...stats.values()].map((stat) => [stat.pid, stat.ppid]));
  let cpuMs = 0;
  let pssBytes = 0;
  for (const pid of descendants(root, parents)) {
    const stat = stats.get(pid);
    cpuMs += stat === undefined ? 0 : stat.userMs + stat.systemMs;
    const rollup = await maybeRead(`/proc/${pid}/smaps_rollup`);
    if (rollup !== null) {
      const memory = parseSmapsRollup(rollup);
      pssBytes += memory.pssAnonBytes + memory.swapPssBytes;
    }
  }
  return { cpuMs, pssBytes };
}

async function psTreeRss(root: number): Promise<number> {
  const { stdout } = await exec("ps", ["-axo", "pid=,ppid=,rss="], {
    timeout: 2000,
    maxBuffer: 4_000_000,
  });
  const rows = stdout
    .trim()
    .split("\n")
    .map((line) => line.trim().split(/\s+/).map(Number));
  const parents = new Map(rows.map(([pid, ppid]) => [pid ?? 0, ppid ?? 0]));
  const tree = descendants(root, parents);
  return rows
    .filter(([pid]) => tree.has(pid ?? 0))
    .reduce((sum, [, , rss]) => sum + (rss ?? 0) * 1024, 0);
}

/** A running sampler of one process tree. */
export interface TreeSampler {
  /** Stops sampling, takes a last sample and returns the window's usage. */
  stop(): Promise<TreeUsage>;
}

/**
 * Samples the process tree under `pid` every `intervalMs` with the probes this
 * host has. A sample that fails leaves the figures of the others.
 */
export function sampleProcessTree(input: {
  readonly pid: number;
  readonly probes: ReadonlySet<Probe>;
  readonly intervalMs: number;
}): TreeSampler {
  const { pid, probes } = input;
  const proc = probes.has("proc_stat") || probes.has("smaps_rollup");
  let cpuMs: number | null = null;
  let peakPssBytes: number | null = null;
  let peakRssBytes: number | null = null;
  let pending: Promise<void> | undefined;

  const sample = async (): Promise<void> => {
    if (proc) {
      try {
        const tree = await procTree(pid);
        cpuMs = probes.has("proc_stat") ? Math.max(cpuMs ?? 0, tree.cpuMs) : null;
        peakPssBytes = probes.has("smaps_rollup")
          ? Math.max(peakPssBytes ?? 0, tree.pssBytes)
          : null;
      } catch {
        // `/proc` could not be listed this time; the next sample tries again.
      }
    }
    if (probes.has("ps")) {
      try {
        peakRssBytes = Math.max(peakRssBytes ?? 0, await psTreeRss(pid));
      } catch {
        // `ps` failed this time; the next sample tries again.
      }
    }
  };
  const tick = (): void => {
    pending ??= sample().finally(() => {
      pending = undefined;
    });
  };
  const timer = setInterval(tick, input.intervalMs);
  timer.unref();
  tick();

  return {
    async stop() {
      clearInterval(timer);
      await pending;
      await sample();
      return { cpuMs, peakPssBytes, peakRssBytes };
    },
  };
}
