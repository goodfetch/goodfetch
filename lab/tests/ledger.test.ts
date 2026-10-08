import { test } from 'node:test';
import assert from 'node:assert/strict';
import { buildLedger, type Classification } from '../tools/ledger.ts';
import type { ImportedRun } from '../tools/import-legacy.ts';
import { validateDocument } from '../src/core/schema.ts';

const counts = { verified: 1, failed: 0, timeout: 0, crashed: 0, aborted: 0, unsupported: 0, rejected: 0, unavailable: 0, skipped: 0 };
const run = (runId: string): ImportedRun => ({
  schemaVersion: 2, runId, lane: 'runtime', campaign: { id: 'runtime-native', source: null }, startedAt: '2026-10-08T00:00:00.000Z', decisionIds: [],
  provenance: { git: { available: false, sha: null, dirty: null }, sourceTreeSha256: null, sourceChanges: 0, binaryChanges: 0, toolchains: {}, integrity: { status: 'verified', checked: 1, mismatches: [] } },
  host: { classRequested: null, classGranted: 'local-dev', os: 'darwin', arch: 'arm64', cpu: null, logicalCpus: null, memoryBytes: null, preflight: null, postflight: null },
  status: 'complete', countUnit: 'cell', cells: [], totals: counts, metricAvailability: {},
  legacy: { shape: 'cells', sourceDir: `results/${runId}`, errorCodeInferred: false, note: null }
});
const entry = (overrides: Partial<Classification['runs'][string]> = {}): Classification['runs'][string] => ({
  role: 'selected', reason: 'cited result', supersededBy: null, decisionIds: ['D-core-language'],
  validity: [{ scope: 'all', class: 'B', note: 'shared host' }], noisyHost: false, rerun: { required: false, priority: null, hostClass: null }, ...overrides
});
const decisions = ['D-core-language', 'D-storage'];

test('every run needs a classification and every classified run must exist', () => {
  const { errors } = buildLedger({ runs: [run('a'), run('b')], classification: { schemaVersion: 1, runs: { a: entry(), ghost: entry() } }, citations: { a: ['reports/x.md'] }, decisions });
  assert.ok(errors.some(e => e.includes('b') && e.includes('no classification')), errors.join('\n'));
  assert.ok(errors.some(e => e.includes('ghost') && e.includes('not a result run')), errors.join('\n'));
});

test('a selected run must be cited, and a superseded run must point to an existing replacement', () => {
  const { errors } = buildLedger({
    runs: [run('a'), run('b')],
    classification: { schemaVersion: 1, runs: { a: entry(), b: entry({ role: 'superseded', supersededBy: 'missing-run' }) } },
    citations: {}, decisions
  });
  assert.ok(errors.some(e => e.includes('a') && e.includes('not cited')), errors.join('\n'));
  assert.ok(errors.some(e => e.includes('b') && e.includes('missing-run')), errors.join('\n'));
});

test('classification overlays decisions and noisy-host status onto a valid ledger', () => {
  const { errors, ledger, runs } = buildLedger({
    runs: [run('a'), run('b')],
    classification: { schemaVersion: 1, runs: { a: entry({ noisyHost: true, rerun: { required: true, priority: 'P0', hostClass: 'reference' } }), b: entry({ role: 'diagnostic', reason: 'interrupted build', decisionIds: ['D-storage'] }) } },
    citations: { a: ['reports/x.md'] }, decisions
  });
  assert.deepEqual(errors, []);
  assert.deepEqual(validateDocument('ledger', ledger), []);
  assert.equal(runs.find(r => r.runId === 'a')!.status, 'noisy-host');
  assert.deepEqual(runs.find(r => r.runId === 'b')!.decisionIds, ['D-storage']);
  assert.equal(ledger.entries.find(e => e.runId === 'a')!.rerun.priority, 'P0');
});

test('decision ids in the classification must exist in the registry', () => {
  const { errors } = buildLedger({ runs: [run('a')], classification: { schemaVersion: 1, runs: { a: entry({ decisionIds: ['D-nope'] }) } }, citations: { a: ['r.md'] }, decisions });
  assert.ok(errors.some(e => e.includes('D-nope')), errors.join('\n'));
});
