import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
const exec = promisify(execFile);
export function sampleProcessGroup(pid: number, intervalMs = 250) {
  let busy = false; let peakRssBytes = 0; let peakProcessCount = 0; let samples = 0; let unavailable = false;
  const sample = async () => {
    if (busy) return; busy = true;
    try {
      const result = await exec('ps', ['-axo', 'pid=,ppid=,pgid=,rss='], { timeout: 2000, maxBuffer: 4_000_000 });
      const rows = result.stdout.trim().split('\n').map(line => line.trim().split(/\s+/).map(Number));
      const selected = new Set([pid]); let changed = true;
      while (changed) { changed = false; for (const row of rows) if (selected.has(row[1]!) && !selected.has(row[0]!)) { selected.add(row[0]!); changed = true; } }
      const owned = rows.filter(row => selected.has(row[0]!) || row[2] === pid);
      peakRssBytes = Math.max(peakRssBytes, owned.reduce((sum, row) => sum + row[3]! * 1024, 0));
      peakProcessCount = Math.max(peakProcessCount, owned.length); samples++;
    } catch { unavailable = true; } finally { busy = false; }
  };
  const timer = setInterval(() => { void sample(); }, intervalMs); timer.unref(); void sample();
  return { stop() { clearInterval(timer); return { peakRssBytes: samples ? peakRssBytes : null, peakProcessCount, samples, unavailable,
    method: 'sampled sum RSS of process group and descendants; shared pages double-counted; short peaks may be missed', intervalMs }; } };
}
