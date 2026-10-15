import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { startFixtures } from '../../fixtures/server.ts';
import { WorkerClient } from '../../src/worker-client.ts';
import { commandFor, labRoot } from '../../src/registry.ts';
import { browserPath } from '../../src/environment.ts';
import { runEngine } from '../../src/core/engine.ts';
import type { BenchCampaign } from '../../src/core/campaign.ts';
import type { HostSnapshot } from '../../src/core/host.ts';

const rustBuilt = existsSync(join(labRoot, 'adapters/cdp-rust/target/release/goodfetch-lab-cdp'));
const cases: [string, string, 'native' | 'guarded'][] = [['js-catalog', 'normal', 'native'], ['form', 'delayed', 'guarded'], ['shadow', 'normal', 'guarded'], ['clicks', 'normal', 'guarded'],
  ...['normal', 'overlay', 'delayed', 'hidden', 'moving', 'rerender', 'never-enabled', 'never-uncovered', 'ambiguous'].map(variant => ['actionability', variant, 'guarded'] as [string, string, 'guarded'])];

async function driverRun(backend: 'node-cdp' | 'rust-cdp') {
  const fixtures = await startFixtures(); const worker = new WorkerClient(commandFor(backend, 20_000));
  const rows: { key: string; status: string; cdpCommands: number | undefined }[] = [];
  try {
    await worker.call('hello', {}); await worker.call('prepare', { executablePath: browserPath(), headless: true }, 45_000);
    for (const [scenario, variant, actionPolicy] of cases) {
      const fixture = await fixtures.createRun({ scenario: scenario as never, seed: 7, pages: 2, actions: 3, variant: variant as never });
      const output = await worker.call('run', { scenario, seed: 7, pages: 2, actions: 3, timeoutMs: 2000, variant, actionPolicy, url: fixture.url }) as { metrics?: Record<string, number> };
      rows.push({ key: `${scenario}/${variant}/${actionPolicy}`, status: (await fixtures.verify(fixture.id, output as never)).status, cdpCommands: output.metrics?.cdpCommands });
    }
  } finally { await worker.close(); await fixtures.close(); }
  return rows;
}

test('the Rust and Node CDP drivers pass the guarded matrix with the same CDP command count for every task', { skip: !rustBuilt && 'run pnpm bootstrap to build adapters/cdp-rust' }, async () => {
  const [node, rust] = [await driverRun('node-cdp'), await driverRun('rust-cdp')];
  for (const rows of [node, rust]) assert.deepEqual(rows.filter(row => row.status !== 'verified'), [], 'every task verified, negatives refused without effect');
  assert.deepEqual(rust.map(row => [row.key, row.cdpCommands]), node.map(row => [row.key, row.cdpCommands]));
});

test('a driver-browser profile shapes the link between each driver and the harness browser, whose resources count for the driver', async () => {
  const quiet: HostSnapshot = { os: process.platform, arch: process.arch, cpu: 'test', logicalCpus: 8, memoryBytes: 1e9, loadAverage1m: 0, memoryFreePercent: 80, thermalWarning: false, onAC: true, lowPowerMode: false, foreignCpuPercent: 0, foreignTop: [], cpuGovernor: 'performance', virtualized: false };
  const campaign: BenchCampaign = { schemaVersion: 2, id: 'driver-link-contract', lane: 'bench', decisionIds: [], hostClass: { requested: 'local-dev', allowDowngrade: false }, seed: 9, blocks: 1, warmup: 0, iterations: 2,
    concurrency: 1, mode: 'closed', timeoutMs: 20_000, maxDurationMs: 120_000, maxRestarts: 1, backends: ['node-cdp', 'playwright'], scenarios: ['form'], fixtures: { pages: 1, actions: 1 },
    browserHost: 'harness', network: { link: 'driver-browser', rttMs: 30 }, actionPolicies: { 'node-cdp': 'guarded', playwright: 'native' } };
  const { run, attempts } = await runEngine(campaign, { outputRoot: await mkdtemp(join(tmpdir(), 'goodfetch-driver-')), skipProvenance: true, hostSnapshot: quiet });
  assert.equal(run.network!.link, 'driver-browser');
  assert.equal(run.network!.calibration.withinTolerance, true);
  assert.deepEqual(attempts.filter(a => a.status !== 'verified').map(a => a.reasons), []);
  // The guarded form sends well over ten CDP commands in sequence, each paying the shaped round trip.
  assert.ok(attempts.filter(a => a.cellId === 'node-cdp:form' && a.phase === 'measure').every(a => a.timing.taskMs! >= 10 * 30));
  const cell = (backend: string) => run.cells.find(c => c.factors.backend === backend)!;
  assert.deepEqual(cell('node-cdp').factors, { backend: 'node-cdp', scenario: 'form', block: 0, policy: 'guarded', rttMs: 30, browserHost: 'harness' });
  assert.equal(typeof cell('node-cdp').metrics['cdp-commands-per-task']!.value, 'number');
  assert.equal(cell('playwright').metrics['cdp-commands-per-task']!.value, null);
});
