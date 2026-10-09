import { test } from 'node:test';
import assert from 'node:assert/strict';
import { decide, evaluateDecision } from '../src/stats/verdict.ts';
import { loadRegistry } from '../src/core/decisions.ts';

const superiority = { kind: 'superiority' as const, alpha: 0.05, power: 0.8 };
const passing = [{ id: 'oracle-exact', ok: true }];
// Per-block ln(option / baseline) built from ratios with a small deterministic jitter.
const blocks = (ratio: number, n: number, jitter = 0.01) => Array.from({ length: n }, (_, i) => Math.log(ratio) + (i % 2 ? jitter : -jitter) * ((i % 3) + 1) / 3);
const base = { direction: 'lower' as const, rule: superiority, delta: 0.1, gates: passing, seed: 11 };

test('a failed contract gate blocks the verdict whatever the measured effect', () => {
  assert.equal(decide({ ...base, logRatios: blocks(0.5, 12), gates: [{ id: 'oracle-exact', ok: false }] }).outcome, 'blocked-by-contract');
});

test('too few blocks, or too much spread for the declared margin, is insufficient-n', () => {
  assert.equal(decide({ ...base, logRatios: blocks(0.5, 3) }).outcome, 'insufficient-n');
  const noisy = decide({ ...base, logRatios: [0.5, -0.5, 0.5, -0.5, 0.5, -0.5, 0.5, -0.5] });
  assert.equal(noisy.outcome, 'insufficient-n');
  assert.ok(noisy.requiredN! > 8);
});

test('a consistent large reduction of a lower-is-better metric is a better outcome', () => {
  const verdict = decide({ ...base, logRatios: blocks(0.5, 12) });
  assert.equal(verdict.outcome, 'better');
  assert.ok(Math.abs(verdict.estimate!.ratio - 0.5) < 0.02);
});

test('a consistent large increase of a lower-is-better metric is a worse outcome', () => {
  assert.equal(decide({ ...base, logRatios: blocks(1.6, 12) }).outcome, 'worse');
});

test('no effect inside the margin is equivalent, a small but real one is trivial', () => {
  assert.equal(decide({ ...base, logRatios: blocks(1, 12, 0.01) }).outcome, 'equivalent');
  assert.equal(decide({ ...base, logRatios: blocks(1.03, 12, 0.002) }).outcome, 'trivial');
});

test('non-inferiority accepts a slightly worse option within the margin', () => {
  assert.equal(decide({ ...base, rule: { ...superiority, kind: 'non-inferiority' }, logRatios: blocks(1.02, 12, 0.003) }).outcome, 'non-inferior');
});

test('on the recorded evidence, D-core-language is mechanically better for Rust and eligible at local-controlled', async () => {
  const registry = loadRegistry();
  const verdict = await evaluateDecision(registry.decisions.find(d => d.id === 'D-core-language')!, { registry });
  assert.equal(verdict.outcome, 'better');
  assert.equal(verdict.host.observed, 'local-controlled');
  assert.equal(verdict.host.eligible, true);
  assert.ok(verdict.n >= verdict.requiredN!, 'enough paired blocks for the declared margin');
});

// A synthetic lab root: one bench run (measurement) and one actionability run (gates) for a driver decision.
async function driverFixture(actionabilityStatus: (backend: string, variant: string) => string, noisyBlocks: number[] = []) {
  const { mkdtemp, mkdir, writeFile } = await import('node:fs/promises'); const { tmpdir } = await import('node:os'); const { join } = await import('node:path');
  const root = await mkdtemp(join(tmpdir(), 'automan-verdict-')); await mkdir(join(root, 'ledger/imported'), { recursive: true });
  const ratio: Record<string, number> = { 'rust-cdp': 0.5, 'node-cdp': 0.9, playwright: 1 };
  const policy: Record<string, string> = { 'rust-cdp': 'guarded', 'node-cdp': 'guarded', playwright: 'native' };
  const bench = { runId: 'bench-1', lane: 'bench', host: { classGranted: 'local-controlled' }, blocks: Array.from({ length: 6 }, (_, index) => ({ index, noisy: noisyBlocks.includes(index) })), cells: Array.from({ length: 6 }, (_, block) => Object.keys(ratio).map(backend => ({
    factors: { backend, scenario: 'form', block, policy: policy[backend], rttMs: 0, browserHost: 'harness' }, status: 'verified',
    metrics: { 'task-p50-ms-verified': { value: 100 * ratio[backend]! * (1 + (block % 2 ? 0.01 : -0.01)) } } }))).flat() };
  const actionability = { runId: 'act-1', lane: 'actionability', host: { classGranted: 'local-controlled' }, cells: Object.keys(ratio).flatMap(backend => ['normal', 'never-enabled'].map(variant => ({
    factors: { backend, variant, policy: policy[backend], block: 0 }, status: actionabilityStatus(backend, variant), metrics: {} }))).filter(cell => cell.status !== 'absent') };
  for (const run of [bench, actionability]) await writeFile(join(root, `ledger/imported/${run.runId}.json`), JSON.stringify(run));
  await writeFile(join(root, 'ledger/results-ledger.json'), JSON.stringify({ entries: [bench, actionability].map(run => ({ runId: run.runId, imported: `ledger/imported/${run.runId}.json` })) }));
  const registry = loadRegistry();
  const decision = { ...registry.decisions.find(d => d.id === 'D-browser-driver')!, evidence: [{ kind: 'run' as const, ref: 'bench-1' }, { kind: 'run' as const, ref: 'act-1' }],
    analysis: { lane: 'bench', options: { 'rust-cdp-guarded': { backend: 'rust-cdp', policy: 'guarded' }, 'node-cdp-guarded': { backend: 'node-cdp', policy: 'guarded' }, 'playwright-native': { backend: 'playwright', policy: 'native' } },
      pairBy: ['scenario', 'rttMs'], stratum: { rttMs: 0 }, gateLanes: { 'actionability-conformance': 'actionability', 'false-success-count': 'actionability' } } };
  return { root, registry, decision };
}

test('driver gates are read from the actionability matrix, not from the measurement lane', async () => {
  const { root, registry, decision } = await driverFixture(() => 'verified');
  const verdict = await evaluateDecision(decision, { registry, root });
  assert.deepEqual(verdict.gates.map(gate => [gate.id, gate.ok, gate.evaluated]), [['actionability-conformance', true, true], ['false-success-count', true, true], ['oracle-exact', true, true]]);
  assert.equal(verdict.option, 'rust-cdp-guarded');
  assert.equal(verdict.outcome, 'better');
});

test('a false success on a negative variant blocks the option whatever its speed', async () => {
  const { root, registry, decision } = await driverFixture((backend, variant) => backend === 'rust-cdp' && variant === 'never-enabled' ? 'failed' : 'verified');
  const verdict = await evaluateDecision(decision, { registry, root });
  assert.equal(verdict.outcome, 'blocked-by-contract');
  assert.equal(verdict.gates.find(gate => gate.id === 'false-success-count')!.ok, false);
});

test('every analysed option is compared with the baseline on its own gates', async () => {
  const { root, registry, decision } = await driverFixture((backend, variant) => backend === 'rust-cdp' && variant === 'never-enabled' ? 'failed' : 'verified');
  const verdict = await evaluateDecision(decision, { registry, root });
  assert.deepEqual(verdict.comparisons.map(c => [c.option, c.outcome]), [['rust-cdp-guarded', 'blocked-by-contract'], ['node-cdp-guarded', 'better']]);
});

test('an option absent from the gate lane fails its gates even when the baseline passes them', async () => {
  const { root, registry, decision } = await driverFixture(backend => backend === 'rust-cdp' ? 'absent' : 'verified');
  const verdict = await evaluateDecision(decision, { registry, root });
  assert.equal(verdict.option, 'rust-cdp-guarded');
  assert.equal(verdict.outcome, 'blocked-by-contract');
  assert.equal(verdict.gates.find(gate => gate.id === 'actionability-conformance')!.ok, false);
});

test('blocks flagged noisy are excluded from the paired estimate when the rule says so', async () => {
  const { root, registry, decision } = await driverFixture(() => 'verified', [0, 3]);
  const verdict = await evaluateDecision(decision, { registry, root });
  assert.equal(decision.rule!.noisyBlockPolicy, 'exclude');
  assert.equal(verdict.n, 4);
});

test('declared strata each get their own comparison, so opposite effects are not pooled', async () => {
  const { root, registry, decision } = await driverFixture(() => 'verified');
  const verdict = await evaluateDecision({ ...decision, analysis: { ...decision.analysis, strata: [{ scenario: 'form', rttMs: 0 }, { scenario: 'clicks', rttMs: 0 }] } }, { registry, root });
  assert.deepEqual(verdict.comparisons.map(c => [c.option, JSON.stringify(c.stratum), c.n]), [
    ['rust-cdp-guarded', '{"scenario":"form","rttMs":0}', 6], ['rust-cdp-guarded', '{"scenario":"clicks","rttMs":0}', 0],
    ['node-cdp-guarded', '{"scenario":"form","rttMs":0}', 6], ['node-cdp-guarded', '{"scenario":"clicks","rttMs":0}', 0]]);
});

// A single-option decision is judged on its contract alone: each gate reads its own lane, and one failed recovery cell blocks it.
async function contractFixture(recoveryStatus: string) {
  const { mkdtemp, mkdir, writeFile } = await import('node:fs/promises'); const { tmpdir } = await import('node:os'); const { join } = await import('node:path');
  const root = await mkdtemp(join(tmpdir(), 'automan-contract-')); await mkdir(join(root, 'ledger/imported'), { recursive: true });
  const runtime = { runId: 'rt-1', lane: 'runtime', host: { classGranted: 'local-dev' }, cells: ['none', 'after-commit'].map((fault, block) => ({ factors: { runtime: 'rust', fault, block }, status: fault === 'none' ? 'verified' : recoveryStatus, metrics: {} })) };
  const admission = { runId: 'adm-1', lane: 'admission', host: { classGranted: 'local-dev' }, cells: [{ factors: { policy: 'fair-domain', fault: 'crash', block: 0 }, status: 'verified', metrics: {} }] };
  for (const run of [runtime, admission]) await writeFile(join(root, `ledger/imported/${run.runId}.json`), JSON.stringify(run));
  await writeFile(join(root, 'ledger/results-ledger.json'), JSON.stringify({ entries: [runtime, admission].map(run => ({ runId: run.runId, imported: `ledger/imported/${run.runId}.json` })) }));
  const registry = loadRegistry();
  const decision = { ...registry.decisions.find(d => d.id === 'D-durable-runtime')!, evidence: [{ kind: 'run' as const, ref: 'rt-1' }, { kind: 'run' as const, ref: 'adm-1' }],
    analysis: { lane: 'runtime', options: { 'journal-leases-epoch': {} }, pairBy: [], stratum: {}, gateLanes: { 'admission-no-loss': 'admission' } } };
  return { root, registry, decision };
}

test('a decision with one option gets a contract verdict, gate by gate in each lane', async () => {
  const met = await contractFixture('verified');
  const verdict = await evaluateDecision(met.decision, { registry: met.registry, root: met.root });
  assert.equal(verdict.outcome, 'contract-met');
  assert.deepEqual(verdict.gates.map(gate => [gate.id, gate.ok]), [['admission-no-loss', true], ['recovery-exact', true], ['oracle-exact', true]]);
  const broken = await contractFixture('failed');
  assert.equal((await evaluateDecision(broken.decision, { registry: broken.registry, root: broken.root })).outcome, 'blocked-by-contract');
});
