import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runLaneCampaign, type LaneCampaign, type LaneDriver } from '../src/core/lanes.ts';
import { validateDocument } from '../src/core/schema.ts';
import type { HostSnapshot } from '../src/core/host.ts';

const quiet: HostSnapshot = { os: process.platform, arch: process.arch, cpu: 'test', logicalCpus: 8, memoryBytes: 1e9, loadAverage1m: 0, memoryFreePercent: 80, thermalWarning: false, onAC: true, lowPowerMode: false, foreignCpuPercent: 0, foreignTop: [], cpuGovernor: 'performance', virtualized: false };
const options = async () => ({ outputRoot: await mkdtemp(join(tmpdir(), 'automan-lane-')), skipProvenance: true, hostSnapshot: quiet });
const campaign = (lane: LaneCampaign['lane'], config: Record<string, unknown>): LaneCampaign => ({ schemaVersion: 2, id: `lane-${lane}`, lane, decisionIds: ['D-storage'], hostClass: { requested: 'local-dev', allowDowngrade: false }, config });

// A driver whose cells report what a lane executor can return, including a crash of the executor itself.
const fake: LaneDriver = {
  validate: () => {}, blocks: () => 1, seed: () => 1,
  combinations: () => [{ name: 'ok' }, { name: 'refuted' }, { name: 'unsupported' }, { name: 'no-start' }, { name: 'throws' }],
  label: cell => String(cell.name),
  runCell: async (_config, cell) => {
    if (cell.name === 'throws') throw new Error('executor exploded');
    const status = { ok: 'verified', refuted: 'failed', unsupported: 'unsupported', 'no-start': 'startup-failed' }[String(cell.name)];
    return { backend: String(cell.name), status, verifiedRowsPerSecond: cell.name === 'ok' ? 100 : null };
  }
};

test('a lane run maps executor outcomes to typed statuses and never counts a crash as verified', async () => {
  const { run, directory } = await runLaneCampaign(campaign('storage', {}), { ...await options(), driver: fake });
  assert.deepEqual(validateDocument('run.v2', run), []);
  const byBackend = new Map(run.cells.map(cell => [cell.factors.backend, cell]));
  assert.equal(byBackend.get('ok')!.status, 'verified');
  assert.equal(byBackend.get('refuted')!.status, 'failed');
  assert.equal(byBackend.get('unsupported')!.status, 'unsupported');
  assert.equal(byBackend.get('no-start')!.errorCode, 'E_STARTUP');
  const exploded = run.cells.find(cell => cell.errorCode === 'E_LANE_EXCEPTION')!;
  assert.equal(exploded.status, 'failed');
  assert.deepEqual([run.totals.verified, run.totals.failed, run.totals.unsupported, run.totals.crashed], [1, 2, 1, 1]);
  const raw = (await readFile(join(directory, 'cells.raw.jsonl'), 'utf8')).trim().split('\n');
  assert.equal(raw.length, 5);
});

test('the storage lane runs a real cell through the engine and verifies it after re-reading the sink', async () => {
  const config = { name: 'lane-storage', engines: ['node'], formats: ['jsonl-durable', 'json-snapshot-durable'], blocks: 1, batches: 4, batchSizes: [8], payloadBytes: 64, writers: [1, 2], producers: 2, timeoutMs: 5000, maxDurationMs: 20_000, busyTimeoutMs: 1000, seed: 3 };
  const { run } = await runLaneCampaign(campaign('storage', config), await options());
  assert.deepEqual(validateDocument('run.v2', run), []);
  assert.equal(run.countUnit, 'cell');
  assert.deepEqual([run.totals.verified, run.totals.unsupported, run.totals.failed], [3, 1, 0], 'snapshot with two writers is unsupported');
  assert.ok(run.cells.filter(cell => cell.status === 'verified').every(cell => cell.metrics['verified-rows-per-second']!.value! > 0));
});
