import { readFileSync } from 'node:fs';
import { readFile } from 'node:fs/promises';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { cpus, totalmem, loadavg, platform, arch } from 'node:os';
import { join } from 'node:path';
import { labRoot } from '../registry.ts';

const exec = promisify(execFile);
export const hostClasses = ['local-dev', 'local-controlled', 'reference'] as const;
export type HostClass = typeof hostClasses[number];
export type HostSnapshot = {
  os: string; arch: string; cpu: string | null; logicalCpus: number; memoryBytes: number; loadAverage1m: number;
  memoryFreePercent: number | null; thermalWarning: boolean | null; onAC: boolean | null; lowPowerMode: boolean | null;
  foreignCpuPercent: number | null; foreignTop: { command: string; cpuPercent: number }[]; cpuGovernor: string | null; virtualized: boolean | null;
  // CPU of the agent app driving the run, recorded apart: it is part of the setup, not a foreign load.
  controllerCpuPercent?: number | null;
};
type Gate = { id: string; equals?: string | number | boolean; max?: number; min?: number };
type HostClassSpec = { class: HostClass; description: string; gates: Gate[] };
export type GateResult = { class: HostClass; id: string; ok: boolean; value: string | number | boolean | null; limit: string };
export type HostAssessment = { classRequested: HostClass; classGranted: HostClass | null; refused: boolean; gates: GateResult[] };

export function loadHostClasses(root = labRoot): HostClassSpec[] {
  return ['local-controlled', 'reference'].map(name => JSON.parse(readFileSync(join(root, 'hosts', `${name}.json`), 'utf8')) as HostClassSpec);
}

function gateValue(id: string, snapshot: HostSnapshot, gitClean: boolean): string | number | boolean | null {
  switch (id) {
    case 'git-clean': return gitClean;
    case 'os': return snapshot.os;
    case 'virtualized': return snapshot.virtualized;
    case 'cpu-governor': return snapshot.cpuGovernor;
    case 'load-per-cpu-1m': return snapshot.logicalCpus ? snapshot.loadAverage1m / snapshot.logicalCpus : null;
    case 'memory-free-percent': return snapshot.memoryFreePercent;
    case 'thermal-warning': return snapshot.thermalWarning;
    case 'on-ac': return snapshot.onAC;
    case 'low-power-mode': return snapshot.lowPowerMode;
    case 'foreign-cpu-percent': return snapshot.foreignCpuPercent;
    default: throw new Error(`unknown host gate ${id}`);
  }
}

// A gate whose value cannot be measured fails: a quiet host must be shown, not assumed.
function evaluate(spec: HostClassSpec, snapshot: HostSnapshot, gitClean: boolean): GateResult[] {
  return spec.gates.map(gate => {
    const value = gateValue(gate.id, snapshot, gitClean);
    const limit = gate.equals !== undefined ? `= ${gate.equals}` : gate.max !== undefined ? `<= ${gate.max}` : `>= ${gate.min}`;
    const ok = value !== null && (gate.equals !== undefined ? value === gate.equals : typeof value === 'number' && (gate.max === undefined || value <= gate.max) && (gate.min === undefined || value >= gate.min));
    return { class: spec.class, id: gate.id, ok, value, limit };
  });
}

export function assessHost({ snapshot, requested, allowDowngrade, gitClean, classes }: { snapshot: HostSnapshot; requested: HostClass; allowDowngrade: boolean; gitClean: boolean; classes: HostClassSpec[] }): HostAssessment {
  const gates: GateResult[] = [];
  const order = hostClasses.slice(0, hostClasses.indexOf(requested) + 1).reverse();
  for (const candidate of order) {
    if (candidate === 'local-dev') return { classRequested: requested, classGranted: allowDowngrade || requested === 'local-dev' ? 'local-dev' : null, refused: !(allowDowngrade || requested === 'local-dev'), gates };
    const spec = classes.find(item => item.class === candidate);
    if (!spec) throw new Error(`missing host class specification ${candidate}`);
    const results = evaluate(spec, snapshot, gitClean); gates.push(...results);
    if (results.every(result => result.ok)) return { classRequested: requested, classGranted: candidate, refused: false, gates };
    if (!allowDowngrade) return { classRequested: requested, classGranted: null, refused: true, gates };
  }
  throw new Error('unreachable host class order');
}

type ProcessRow = { pid: number; ppid: number; pgid: number; cpu: number; command: string };
// The agent app that launches the campaign (Claude desktop app and CLI) runs throughout any run it drives.
const controller = /(^|\/)(Claude|Claude Helper( \([A-Za-z]+\))?|claude)$/;
export function classifyProcesses(rows: ProcessRow[], ownProcessGroups: number[]) {
  const own = new Set(ownProcessGroups); let changed = true;
  while (changed) { changed = false; for (const row of rows) if ((own.has(row.ppid) || own.has(row.pgid)) && !own.has(row.pid)) { own.add(row.pid); changed = true; } }
  const busy = rows.filter(row => !own.has(row.pid) && Number.isFinite(row.cpu) && row.cpu > 0);
  const isController = (row: ProcessRow) => controller.test(row.command);
  const foreign = busy.filter(row => !isController(row)).sort((a, b) => b.cpu - a.cpu);
  return { foreignCpuPercent: foreign.reduce((sum, row) => sum + row.cpu, 0), controllerCpuPercent: busy.filter(isController).reduce((sum, row) => sum + row.cpu, 0),
    foreignTop: foreign.slice(0, 5).map(row => ({ command: row.command.split('/').at(-1) ?? row.command, cpuPercent: row.cpu })) };
}

async function run(command: string, args: string[]) { try { return (await exec(command, args, { timeout: 3000, maxBuffer: 4_000_000 })).stdout; } catch { return null; } }

// Snapshot of the current host; values that cannot be read on this platform stay null.
export async function snapshotHost(ownProcessGroups: number[] = [process.pid]): Promise<HostSnapshot> {
  const os = platform();
  const snapshot: HostSnapshot = { os, arch: arch(), cpu: cpus()[0]?.model ?? null, logicalCpus: cpus().length, memoryBytes: totalmem(), loadAverage1m: loadavg()[0] ?? 0,
    memoryFreePercent: null, thermalWarning: null, onAC: null, lowPowerMode: null, foreignCpuPercent: null, foreignTop: [], cpuGovernor: null, virtualized: null };
  const ps = await run('ps', ['-axo', 'pid=,ppid=,pgid=,pcpu=,comm=']);
  if (ps !== null) {
    const rows = ps.trim().split('\n').map(line => line.trim().split(/\s+/)).map(([pid, ppid, pgid, pcpu, ...command]) => ({ pid: Number(pid), ppid: Number(ppid), pgid: Number(pgid), cpu: Number(pcpu), command: command.join(' ') }));
    Object.assign(snapshot, classifyProcesses(rows, ownProcessGroups));
  }
  if (os === 'darwin') {
    const pressure = await run('memory_pressure', ['-Q']);
    const free = pressure?.match(/free percentage:\s*(\d+)%/i);
    if (free) snapshot.memoryFreePercent = Number(free[1]);
    const thermal = await run('pmset', ['-g', 'therm']);
    if (thermal !== null) snapshot.thermalWarning = !/No thermal warning level has been recorded/i.test(thermal) || !/No performance warning level has been recorded/i.test(thermal);
    const battery = await run('pmset', ['-g', 'batt']);
    if (battery !== null) snapshot.onAC = /AC Power/.test(battery);
    const settings = await run('pmset', ['-g']);
    const low = settings?.match(/lowpowermode\s+(\d)/);
    snapshot.lowPowerMode = low ? low[1] === '1' : settings !== null ? false : null;
    snapshot.virtualized = (await run('sysctl', ['-n', 'kern.hv_vmm_present']))?.trim() === '1';
  } else if (os === 'linux') {
    const meminfo = await readFile('/proc/meminfo', 'utf8').catch(() => null);
    const total = meminfo?.match(/MemTotal:\s+(\d+)/), available = meminfo?.match(/MemAvailable:\s+(\d+)/);
    if (total && available) snapshot.memoryFreePercent = Math.round(Number(available[1]) * 100 / Number(total[1]));
    snapshot.cpuGovernor = (await readFile('/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor', 'utf8').catch(() => null))?.trim() ?? null;
    const virt = await run('systemd-detect-virt', []);
    snapshot.virtualized = virt === null ? null : virt.trim() !== 'none';
    snapshot.onAC = true; snapshot.lowPowerMode = false;
    const thermal = await readFile('/sys/class/thermal/thermal_zone0/temp', 'utf8').catch(() => null);
    snapshot.thermalWarning = thermal === null ? null : Number(thermal) > 90_000;
  }
  return snapshot;
}
