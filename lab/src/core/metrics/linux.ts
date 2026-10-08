// Linux probes for the reference host: process-tree CPU from /proc, proportional memory from smaps_rollup,
// and cgroup v2 counters that also cover exited children of a per-run scope.
import { readFile, readdir } from 'node:fs/promises';

export function parseProcStat(text: string, clockTicksPerSecond: number) {
  const close = text.lastIndexOf(')');
  const fields = text.slice(close + 2).trim().split(/\s+/);
  // Fields after the command: state(3) ppid(4) ... utime(14) stime(15); indices here start at field 3.
  const pid = Number(text.slice(0, text.indexOf(' ')));
  return { pid, ppid: Number(fields[1]), userMs: Number(fields[11]) * 1000 / clockTicksPerSecond, systemMs: Number(fields[12]) * 1000 / clockTicksPerSecond };
}

export function parseSmapsRollup(text: string) {
  const kib = (name: string) => { const match = text.match(new RegExp(`^${name}:\\s+(\\d+) kB`, 'm')); return match ? Number(match[1]) * 1024 : 0; };
  return { pssBytes: kib('Pss'), pssAnonBytes: kib('Pss_Anon'), swapPssBytes: kib('SwapPss') };
}

export function parseCgroupBytes(text: string) {
  const value = text.trim();
  if (!/^\d+$/.test(value)) throw new Error(`cgroup value is not a byte count: ${value}`);
  return Number(value);
}

export function parseCgroupCpuStat(text: string) {
  const field = (name: string) => Number(text.match(new RegExp(`^${name} (\\d+)$`, 'm'))?.[1] ?? NaN);
  return { userMs: field('user_usec') / 1000, systemMs: field('system_usec') / 1000 };
}

// Process tree rooted at `root`: CPU from /proc stat, peak proportional memory summed over the tree (anonymous + swap).
export async function sampleLinuxTree(root: number, clockTicksPerSecond = 100) {
  const parents = new Map<number, number>();
  for (const entry of await readdir('/proc')) {
    if (!/^\d+$/.test(entry)) continue;
    try { const stat = parseProcStat(await readFile(`/proc/${entry}/stat`, 'utf8'), clockTicksPerSecond); parents.set(stat.pid, stat.ppid); } catch { /* process exited */ }
  }
  const tree = new Set([root]); let changed = true;
  while (changed) { changed = false; for (const [pid, ppid] of parents) if (tree.has(ppid) && !tree.has(pid)) { tree.add(pid); changed = true; } }
  let userMs = 0, systemMs = 0, pssBytes = 0;
  for (const pid of tree) {
    try {
      const stat = parseProcStat(await readFile(`/proc/${pid}/stat`, 'utf8'), clockTicksPerSecond); userMs += stat.userMs; systemMs += stat.systemMs;
      const memory = parseSmapsRollup(await readFile(`/proc/${pid}/smaps_rollup`, 'utf8')); pssBytes += memory.pssAnonBytes + memory.swapPssBytes;
    } catch { /* process exited between listing and reading */ }
  }
  return { processes: tree.size, userMs, systemMs, pssBytes };
}

// Periodic sampler with the same shape as the macOS sampler: peak proportional memory and cumulative CPU of the tree.
export function sampleLinuxResources(root: number, intervalMs = 100) {
  let peakPssBytes = 0, userMs = 0, systemMs = 0, samples = 0, unavailable = false, pending: Promise<void> | undefined, stopped = false;
  const sample = async () => { try { const tree = await sampleLinuxTree(root); samples++; peakPssBytes = Math.max(peakPssBytes, tree.pssBytes); userMs = Math.max(userMs, tree.userMs); systemMs = Math.max(systemMs, tree.systemMs); } catch { unavailable = true; } };
  const tick = () => { if (!stopped && !pending) pending = sample().finally(() => { pending = undefined; }); };
  const timer = setInterval(tick, intervalMs); timer.unref(); tick();
  return { async stop() { stopped = true; clearInterval(timer); await pending; await sample();
    return { intervalMs, samples, unavailable, peakPssBytes: samples ? peakPssBytes : null, sampledUserCpuMs: userMs, sampledSystemCpuMs: systemMs, method: 'Linux /proc stat and smaps_rollup over the process tree; CPU of exited children is only covered by a per-run cgroup scope' }; } };
}
