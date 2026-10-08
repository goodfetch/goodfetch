import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { labRoot } from '../../registry.ts';
import type { Metric } from '../decisions.ts';

export type Availability = Record<string, { available: boolean; reason?: string }>;
export type Measure = { value: number | null; unit: string; source: string };

// Sources that the harness or the adapter protocol provide on every platform.
const builtin = new Set(['harness', 'adapter', 'ps']);
// Probes that are planned but not implemented yet are reported as such, never approximated.
const planned: Record<string, string> = { 'protocol-v2-usage': 'adapter protocol v2 usage reporting is not implemented yet', 'harness-pricing': 'pricing tables are not implemented yet', 'vm-hard-reset': 'requires the VM hard-reset harness',
  'cgroup-v2-memory-peak': 'requires a per-run systemd scope on the reference host; the parser exists but the engine does not create the scope yet',
  'cgroup-v2-cpu-stat': 'requires a per-run systemd scope on the reference host; the parser exists but the engine does not create the scope yet' };

export function detectProbes(platform = process.platform): Record<string, boolean> {
  return {
    'proc_pid_rusage-v4': platform === 'darwin' && existsSync(join(labRoot, '.native/process-probe')),
    'smaps_rollup': platform === 'linux' && existsSync('/proc/self/smaps_rollup'),
    'proc-stat': platform === 'linux' && existsSync('/proc/self/stat')
  };
}

export function resolveAvailability(metrics: Metric[], { platform, probes }: { platform: string; probes: Record<string, boolean> }): Availability {
  const availability: Availability = {};
  for (const metric of metrics) {
    const probe = platform === 'darwin' || platform === 'linux' ? metric.platforms[platform] : null;
    if (!probe) availability[metric.id] = { available: false, reason: `no probe on ${platform}` };
    else if (builtin.has(probe)) availability[metric.id] = { available: true };
    else if (planned[probe]) availability[metric.id] = { available: false, reason: planned[probe] };
    else if (probes[probe]) availability[metric.id] = { available: true };
    else availability[metric.id] = { available: false, reason: `probe ${probe} is not installed on this host` };
  }
  return availability;
}

export function measureMetric(availability: Availability, metrics: Metric[], id: string, value: number | null, source: string): Measure {
  const metric = metrics.find(item => item.id === id);
  if (!metric) throw new Error(`unknown metric ${id}`);
  const state = availability[id];
  if (!state?.available) return { value: null, unit: metric.unit, source: `unavailable: ${state?.reason ?? 'not resolved'}` };
  return { value: value !== null && Number.isFinite(value) ? value : null, unit: metric.unit, source };
}
