import { test } from 'node:test';
import assert from 'node:assert/strict';
import { renderDecisionsReport } from '../src/report/generate.ts';
import { loadRegistry, lintEditorial } from '../src/core/decisions.ts';

const registry = loadRegistry();
const verdict = (ratio: number) => ({
  decisionId: 'D-core-language', outcome: 'better', n: 12, requiredN: 6, option: 'rust-tokio', baseline: 'node', metricId: 'cpu-ms-per-verified-job',
  estimate: { ratio, ratioCi: [ratio * 0.9, ratio * 1.1], interval: '95% block bootstrap', wilcoxon: { pValue: 0.0005 } },
  gates: [{ id: 'oracle-exact', ok: true, evaluated: true }], host: { required: 'local-controlled', observed: 'local-controlled', eligible: true }, inputs: { runs: ['run-a'] }
});

test('without verdicts the generated report carries no measured figure', () => {
  const report = renderDecisionsReport(registry, []);
  assert.equal(lintEditorial(report), null);
  assert.match(report, /D-core-language/);
});

test('every figure of a decision comes from its verdict and changes with it', () => {
  const first = renderDecisionsReport(registry, [verdict(0.512)]);
  const second = renderDecisionsReport(registry, [verdict(0.734)]);
  assert.match(first, /0\.512/);
  assert.doesNotMatch(first, /0\.734/);
  assert.match(second, /0\.734/);
  assert.equal(renderDecisionsReport(registry, [verdict(0.512)]), first, 'deterministic');
});
