import { test } from 'node:test';
import assert from 'node:assert/strict';
import { loadRegistry, checkRegistry, type Registry } from '../src/core/decisions.ts';

const technical = (overrides: Record<string, unknown> = {}) => ({
  id: 'D-example', title: 'Example', question: 'Which option?', kind: 'technical', status: 'open',
  options: [{ id: 'a', description: 'A' }, { id: 'b', description: 'B' }], baseline: 'a',
  primaryMetric: { metricId: 'cpu-ms-per-verified-job', blockStatistic: 'ratio-of-sums' },
  secondaryMetrics: [], contractGates: ['oracle-exact'], delta: { kind: 'ratio', value: 0.1 },
  rule: { kind: 'superiority', alpha: 0.05, power: 0.8, noisyBlockPolicy: 'exclude' },
  hostClass: { decision: 'local-controlled', publication: 'reference' },
  networkProfiles: [], campaigns: [], evidence: [], verdictRef: null, adr: null, dependsOn: [], notes: '', ...overrides
});
const registryWith = (decisions: unknown[], claims: unknown[] = []): Registry => {
  const base = loadRegistry();
  return { ...base, decisions: decisions as Registry['decisions'], claims: claims as Registry['claims'] };
};

test('the registry committed in the lab is valid', () => {
  assert.deepEqual(checkRegistry(loadRegistry()), []);
});

test('a technical decision must name a known metric, a baseline among its options and a decision rule', () => {
  const errors = checkRegistry(registryWith([
    technical({ id: 'D-unknown-metric', primaryMetric: { metricId: 'not-a-metric', blockStatistic: 'median' } }),
    technical({ id: 'D-bad-baseline', baseline: 'z' }),
    technical({ id: 'D-no-rule', rule: undefined })
  ]));
  assert.ok(errors.some(e => e.includes('D-unknown-metric') && e.includes('not-a-metric')), errors.join('\n'));
  assert.ok(errors.some(e => e.includes('D-bad-baseline') && e.includes('baseline')), errors.join('\n'));
  assert.ok(errors.some(e => e.includes('D-no-rule')), errors.join('\n'));
});

test('editorial notes cannot carry measured figures, which must come from verdicts', () => {
  const errors = checkRegistry(registryWith([technical({ notes: 'Rust was 49 % faster at 197 jobs/s' })]));
  assert.ok(errors.some(e => e.includes('D-example') && e.includes('notes')), errors.join('\n'));
});

test('references between decisions and claims must resolve', () => {
  const errors = checkRegistry(registryWith(
    [technical({ dependsOn: ['D-missing'] })],
    [{ id: 'C-orphan', statement: 'Orphan claim', decisionId: 'D-nowhere', metricId: 'cpu-ms-per-verified-job', comparator: { option: 'a', against: ['b'] }, direction: 'lower', publicationHostClass: 'reference', status: 'candidate', evidence: [], notes: '' }]
  ));
  assert.ok(errors.some(e => e.includes('D-missing')), errors.join('\n'));
  assert.ok(errors.some(e => e.includes('C-orphan') && e.includes('D-nowhere')), errors.join('\n'));
});
