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

test('on the imported runs, D-core-language is mechanically insufficient-n and not eligible at local-dev', async () => {
  const registry = loadRegistry();
  const verdict = await evaluateDecision(registry.decisions.find(d => d.id === 'D-core-language')!, { registry });
  assert.equal(verdict.outcome, 'insufficient-n');
  assert.equal(verdict.host.observed, 'local-dev');
  assert.equal(verdict.host.eligible, false);
  assert.ok(verdict.n >= 1, 'paired blocks were found in the evidence runs');
});
