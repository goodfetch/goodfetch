// What every v2 run shares, whatever its lane: provenance, host gate, block noise, availability and final status.
import { mkdir, writeFile } from 'node:fs/promises';
import { createHash, randomBytes } from 'node:crypto';
import { join } from 'node:path';
import { labRoot } from '../registry.ts';
import { copySourceSnapshot, sourceFingerprint } from '../environment.ts';
import { LabError } from './errors.ts';
import { assessHost, loadHostClasses, snapshotHost, type HostClass, type HostSnapshot, type HostAssessment } from './host.ts';
import { captureProvenance, type Provenance } from './provenance.ts';
import { loadRegistry } from './decisions.ts';
import { resolveAvailability, detectProbes, measureMetric, type Availability, type Measure } from './metrics/registry.ts';

export type RunOptions = { outputRoot?: string; skipProvenance?: boolean; hostSnapshot?: HostSnapshot; source?: string };
export type BlockRecord = { index: number; order: string[]; noisy: boolean; host: { loadPerCpu1m: number | null; foreignCpuPercent: number | null } };

export async function openRun(campaign: { id: string; hostClass: { requested: HostClass; allowDowngrade: boolean } }, options: RunOptions) {
  const registry = loadRegistry();
  const provenance: Provenance | null = options.skipProvenance ? null : await captureProvenance();
  const gitClean = Boolean(provenance?.git.available && provenance.git.dirty === false);
  const classes = loadHostClasses();
  const preflight = options.hostSnapshot ?? await snapshotHost([process.pid]);
  const assessment: HostAssessment = assessHost({ snapshot: preflight, requested: campaign.hostClass.requested, allowDowngrade: campaign.hostClass.allowDowngrade, gitClean, classes });
  if (assessment.refused) throw new LabError('E_HOST_GATE', `host refused for ${campaign.hostClass.requested}: ${assessment.gates.filter(g => !g.ok).map(g => `${g.id}=${g.value} (${g.limit})`).join(', ')}`);
  const granted = assessment.classGranted!;
  const startedAt = new Date().toISOString();
  const runId = `${campaign.id}-${startedAt.replace(/[:.]/g, '-')}-${randomBytes(3).toString('hex')}`;
  const directory = join(options.outputRoot ?? join(labRoot, 'results'), runId); await mkdir(directory, { recursive: true });
  if (provenance) await copySourceSnapshot(directory, provenance.sourceHashes);
  const probes = detectProbes();
  const availability: Availability = resolveAvailability(registry.metrics, { platform: process.platform, probes });
  const measureOf = (id: string, value: number | null, source: string): Measure => measureMetric(availability, registry.metrics, id, value, source);
  const blocks: BlockRecord[] = [];
  async function sampleBlock(index: number, order: string[]) {
    const snapshot = options.hostSnapshot ?? await snapshotHost([process.pid]);
    const loadPerCpu = snapshot.logicalCpus ? snapshot.loadAverage1m / snapshot.logicalCpus : null;
    const gates = classes.find(spec => spec.class === granted)?.gates ?? [];
    const max = (id: string) => gates.find(gate => gate.id === id)?.max;
    const over = (limit: number | undefined, value: number | null) => limit !== undefined && (value === null || value > limit);
    const noisy = granted !== 'local-dev' && (over(max('load-per-cpu-1m'), loadPerCpu) || over(max('foreign-cpu-percent'), snapshot.foreignCpuPercent));
    blocks.push({ index, order, noisy, host: { loadPerCpu1m: loadPerCpu, foreignCpuPercent: snapshot.foreignCpuPercent } });
  }
  // Writes run.v2.json; a run whose sources changed is invalid, a run with a noisy block is flagged.
  async function close<Cell extends { counts: Record<string, number> }>(fields: { lane: string; campaign: object; decisionIds: string[]; countUnit: 'attempt' | 'cell'; cells: Cell[]; binaryChanges: number | null }) {
    const postflight = options.hostSnapshot ?? await snapshotHost([process.pid]);
    const sourceChanges = provenance ? Object.entries(await sourceFingerprint()).filter(([path, hash]) => provenance.sourceHashes[path] !== hash).length : null;
    const totals = { verified: 0, failed: 0, timeout: 0, crashed: 0, aborted: 0, unsupported: 0, rejected: 0, unavailable: 0, skipped: 0 };
    for (const cell of fields.cells) for (const key of Object.keys(totals) as (keyof typeof totals)[]) totals[key] += cell.counts[key] ?? 0;
    const status = (sourceChanges ?? 0) > 0 || (fields.binaryChanges ?? 0) > 0 ? 'invalid-provenance' : blocks.some(block => block.noisy) ? 'noisy-host' : 'complete';
    const run = {
      schemaVersion: 2 as const, runId, lane: fields.lane,
      campaign: { id: campaign.id, source: options.source ?? null, sha256: createHash('sha256').update(JSON.stringify(fields.campaign)).digest('hex') },
      startedAt, decisionIds: fields.decisionIds,
      provenance: { git: provenance?.git ?? { available: false, sha: null, dirty: null }, sourceTreeSha256: provenance?.sourceTreeSha256 ?? null, sourceChanges, binaryChanges: fields.binaryChanges,
        toolchains: provenance?.toolchains ?? { node: process.version }, integrity: { status: 'uncatalogued' as const, checked: 0, mismatches: [] as string[] } },
      host: { classRequested: campaign.hostClass.requested, classGranted: granted, os: preflight.os, arch: preflight.arch, cpu: preflight.cpu, logicalCpus: preflight.logicalCpus, memoryBytes: preflight.memoryBytes,
        preflight: { snapshot: preflight, gates: assessment.gates }, postflight: { snapshot: postflight } },
      status, countUnit: fields.countUnit, cells: fields.cells, totals, metricAvailability: availability, legacy: null, blocks
    };
    await writeFile(join(directory, 'run.v2.json'), JSON.stringify(run, null, 1) + '\n');
    return run;
  }
  return { runId, directory, granted, availability, measureOf, sampleBlock, close };
}
