import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { calibrate, withinTolerance, networkAvailability } from '../src/net/latency-proxy.ts';
import { runEngine } from '../src/core/engine.ts';
import { commandFor } from '../src/registry.ts';
import { loadRegistry } from '../src/core/decisions.ts';
import type { HostSnapshot } from '../src/core/host.ts';
import type { BenchCampaign } from '../src/core/campaign.ts';

// Holds on any host: a chunk never leaves before its due time, so no shaped round trip is shorter than the profile.
// Precision within the tolerance depends on a quiet host and is checked alone in tests/timing/.
test('a latency profile is measured by calibration, not assumed', async () => {
  for (const rttMs of [0, 20, 40]) {
    const calibration = await calibrate(rttMs, 20);
    assert.equal(calibration.samples, 20);
    assert.ok(calibration.medianMs >= rttMs, `rtt ${rttMs}: shaped median ${calibration.medianMs}`);
  }
});

test('calibration tolerance is the larger of ten percent and two milliseconds', () => {
  assert.equal(withinTolerance(100, 109), true);
  assert.equal(withinTolerance(100, 115), false);
  assert.equal(withinTolerance(0, 1.5), true);
  assert.equal(withinTolerance(0, 3), false);
});

test('an out-of-tolerance network makes timing metrics unavailable but keeps contract metrics', () => {
  const { metrics } = loadRegistry();
  const availability = networkAvailability(metrics, { rttMs: 100, medianMs: 150, p95Ms: 160, baselineMedianMs: 0.1, addedMedianMs: 149.9, samples: 20, withinTolerance: false });
  assert.equal(availability['task-p50-ms-verified']!.available, false);
  assert.match(availability['task-p50-ms-verified']!.reason!, /calibration/);
  assert.equal(availability['oracle-exact'], undefined, 'contract metrics are not overridden');
});

test('the engine applies a browser-origin latency profile and records its calibration', async () => {
  const quiet: HostSnapshot = { os: process.platform, arch: process.arch, cpu: 'test', logicalCpus: 8, memoryBytes: 1e9, loadAverage1m: 0, memoryFreePercent: 80, thermalWarning: false, onAC: true, lowPowerMode: false, foreignCpuPercent: 0, foreignTop: [], cpuGovernor: 'performance', virtualized: false };
  const campaign: BenchCampaign = { schemaVersion: 2, id: 'net-contract', lane: 'bench', decisionIds: [], hostClass: { requested: 'local-dev', allowDowngrade: false }, seed: 5, blocks: 1, warmup: 0, iterations: 3, concurrency: 1, mode: 'closed', timeoutMs: 10_000, maxDurationMs: 60_000, maxRestarts: 1, backends: ['node-http'], scenarios: ['http-catalog'], fixtures: { pages: 2, actions: 1 }, network: { link: 'browser-origin', rttMs: 30 } };
  const { run, attempts } = await runEngine(campaign, { outputRoot: await mkdtemp(join(tmpdir(), 'goodfetch-net-')), skipProvenance: true, hostSnapshot: quiet, command: backend => commandFor(backend as never, 10_000) });
  // Whether the calibration is within tolerance depends on host load; an out-of-tolerance one only withdraws timing metrics.
  assert.equal(run.network!.calibration.rttMs, 30);
  assert.ok(run.network!.calibration.medianMs >= 30, `shaped median ${run.network!.calibration.medianMs}`);
  const measured = attempts.filter(a => a.phase === 'measure' && a.status === 'verified');
  assert.equal(measured.length, 3);
  // Two catalog pages, each one request through the shaped link: at least two round trips.
  assert.ok(measured.every(a => a.timing.taskMs! >= 2 * 30), measured.map(a => a.timing.taskMs).join(','));
});
