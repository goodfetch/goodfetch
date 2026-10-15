import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runEngine } from '../src/core/engine.ts';
import { startOracle, type Oracle } from '../src/core/oracle.ts';
import { runCampaign, type Campaign } from '../src/campaign.ts';
import { fromLegacyBench, type BenchCampaign } from '../src/core/campaign.ts';
import { validateDocument } from '../src/core/schema.ts';
import type { HostSnapshot } from '../src/core/host.ts';
import { commandFor } from '../src/registry.ts';

const quiet: HostSnapshot = { os: process.platform, arch: process.arch, cpu: 'test', logicalCpus: 8, memoryBytes: 1e9, loadAverage1m: 0, memoryFreePercent: 80, thermalWarning: false, onAC: true, lowPowerMode: false, foreignCpuPercent: 0, foreignTop: [], cpuGovernor: 'performance', virtualized: false };
const base = (overrides: Partial<BenchCampaign> = {}): BenchCampaign => ({
  schemaVersion: 2, id: 'engine-contract', lane: 'bench', decisionIds: [], hostClass: { requested: 'local-dev', allowDowngrade: false },
  seed: 42, blocks: 1, warmup: 0, iterations: 2, concurrency: 1, mode: 'closed', timeoutMs: 2000, maxDurationMs: 20_000, maxRestarts: 2,
  backends: ['node-http'], scenarios: ['http-catalog'], fixtures: { pages: 1, actions: 1 }, ...overrides
});
const engine = async (campaign: BenchCampaign, options: Parameters<typeof runEngine>[1] = {}) =>
  runEngine(campaign, { outputRoot: await mkdtemp(join(tmpdir(), 'goodfetch-engine-')), skipProvenance: true, hostSnapshot: quiet, ...options });

// Fake catalog adapter: answers with the fixture's records (it knows their seeded formula), after a delay.
// When `hangFlag` is given, the second run call of the first process hangs, then a flag file prevents hanging again.
function fakeAdapter(delayMs: number, hangFlag?: string) {
  const script = `const fs=require('node:fs');let runs=0;const flag=${JSON.stringify(hangFlag ?? '')};
const records=t=>Array.from({length:t.pages*2},(_,i)=>({id:'P-'+String(i+1).padStart(3,'0'),price:1201+i*37+(t.seed===42?0:t.seed%97)}));
require('node:readline').createInterface({input:process.stdin}).on('line',line=>{const m=JSON.parse(line);const reply=r=>process.stdout.write(JSON.stringify({version:1,id:m.id,result:r})+'\\n');
if(m.method==='hello')return reply({capabilities:['http-catalog'],maxConcurrency:16});
if(m.method==='prepare')return reply({ready:true});
if(m.method==='close'){reply({closed:true});process.exit(0)}
if(m.method==='run'){runs++;if(flag&&runs===2&&!fs.existsSync(flag)){fs.writeFileSync(flag,'1');return}setTimeout(()=>reply({status:'ok',records:records(m.params)}),${delayMs})}})`;
  return () => ({ command: process.execPath, args: ['-e', script], timeoutMs: 700 });
}

test('a failed conformance gate blocks measurement and the plan stays visible as skipped attempts', async () => {
  const empty = `require('node:readline').createInterface({input:process.stdin}).on('line',line=>{const m=JSON.parse(line);const r=m.method==='hello'?{capabilities:['http-catalog']}:m.method==='run'?{status:'ok',records:[]}:{};process.stdout.write(JSON.stringify({version:1,id:m.id,result:r})+'\\n');if(m.method==='close')process.exit(0)})`;
  const { run, attempts } = await engine(base({ iterations: 3 }), { command: () => ({ command: process.execPath, args: ['-e', empty], timeoutMs: 1000 }) });
  assert.deepEqual(validateDocument('run.v2', run), []);
  assert.deepEqual(attempts.map(a => [a.phase, a.status, a.errorCode]), [['conformance', 'failed', 'E_ORACLE_MISMATCH'], ['measure', 'skipped-gate', null], ['measure', 'skipped-gate', null], ['measure', 'skipped-gate', null]]);
  assert.equal(run.totals.verified, 0);
});

test('the engine reproduces the verified and failed counts of the legacy runner for the same seed', async () => {
  const legacy: Campaign = { name: 'parity', backends: ['node-http'], scenarios: ['http-catalog'], seed: 7, blocks: 2, iterations: 4, warmup: 1, concurrency: 2, mode: 'closed', timeoutMs: 5000, maxDurationMs: 20_000, pages: 2, actions: 1 };
  const v1 = await runCampaign(legacy, { outputRoot: await mkdtemp(join(tmpdir(), 'goodfetch-v1-')), skipEnvironment: true, command: backend => commandFor(backend, 5000) });
  const { attempts } = await engine(fromLegacyBench(legacy), { command: backend => commandFor(backend as never, 5000) });
  for (const phase of ['conformance', 'warmup', 'measure'] as const) for (const status of ['verified', 'failed'] as const)
    assert.equal(attempts.filter(a => a.phase === phase && a.status === status).length, v1.rows.filter(r => r.phase === phase && r.status === status).length, `${phase}/${status}`);
});

test('a hung adapter yields one timeout, aborted siblings excluded from failures, then a restarted worker', async () => {
  const flag = join(await mkdtemp(join(tmpdir(), 'goodfetch-hang-')), 'hung');
  const { run, attempts } = await engine(base({ iterations: 14, concurrency: 4, timeoutMs: 700, maxRestarts: 1 }), { command: fakeAdapter(200, flag) });
  const measured = attempts.filter(a => a.phase === 'measure');
  assert.equal(measured.filter(a => a.status === 'timeout').length, 1);
  assert.equal(measured.filter(a => a.status === 'aborted').length, 3);
  assert.ok(measured.filter(a => a.status === 'aborted').every(a => a.errorCode === 'E_SIBLING_ABORT'));
  assert.equal(measured.filter(a => a.status === 'failed' || a.status === 'crashed').length, 0);
  assert.ok(measured.some(a => a.status === 'verified' && a.workerGeneration === 1), 'attempts verified on the restarted worker');
  assert.equal(run.cells[0]!.counts.failed, 0);
});

test('available resource probes measure the adapter process tree, unavailable ones stay null', async () => {
  const { run } = await engine(base({ iterations: 3 }), { command: fakeAdapter(20) });
  const metrics: Record<string, { value: number | null; unit: string }> = run.cells[0]!.metrics;
  for (const id of ['cpu-ms-per-verified-job', 'footprint-peak-bytes']) {
    if (run.metricAvailability[id]!.available) assert.ok(metrics[id]!.value! > 0, `${id} measured`);
    else assert.equal(metrics[id]!.value, null, `${id} unavailable`);
  }
  assert.equal(metrics['task-p50-ms-verified']!.unit, 'ms');
});

test('a slow oracle is timed apart and never inflates the task duration', async () => {
  const real = await startOracle('in-process');
  const slow: Oracle = { ...real, verify: async (id, output) => { await new Promise(resolve => setTimeout(resolve, 250)); return real.verify(id, output); } };
  try {
    const { attempts } = await engine(base({ iterations: 2 }), { command: fakeAdapter(5), oracle: slow });
    for (const attempt of attempts) {
      assert.equal(attempt.status, 'verified');
      assert.ok(attempt.timing.oracleMs! >= 240, `oracle ${attempt.timing.oracleMs}`);
      assert.ok(attempt.timing.taskMs! < 150, `task ${attempt.timing.taskMs}`);
    }
  } finally { await real.close(); }
});
