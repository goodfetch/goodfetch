import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, writeFile, readFile, readdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { importLegacy, writeImported } from '../tools/import-legacy.ts';
import { validateDocument } from '../src/core/schema.ts';

const sha = (text: string) => createHash('sha256').update(text).digest('hex');
const counts = (verified: number, failed = 0, unsupported = 0) => ({ attempts: verified + failed + unsupported, verified, failed, unsupported, timeout: 0, crashed: 0, rejected: 0 });
const env = { node: 'v25.5.0', os: 'darwin', architecture: 'arm64', cpu: 'Apple M3 Max', logicalCpuCount: 14, totalMemoryBytes: 1, browser: { version: 'Chrome 153' }, sourceHashes: { 'src/a.ts': 'aa' } };

async function legacyLab() {
  const root = await mkdtemp(join(tmpdir(), 'goodfetch-import-'));
  const write = async (path: string, value: unknown) => { await mkdir(join(root, path, '..'), { recursive: true }); const text = typeof value === 'string' ? value : JSON.stringify(value); await writeFile(join(root, path), text); return text; };
  const bench = 'qualification-2026-10-07T21-25-08-753Z-aaaaaa';
  const benchSummary = await write(`results/${bench}/summary.json`, { id: bench, sourceChanges: [], summaries: [
    { backend: 'node-http', scenario: 'http-catalog', conformance: counts(4), warmup: counts(1), measurement: counts(2), throughput: { verifiedTasksPerSecond: 10 }, endToEndMs: { n: 2, p50: 1, p95: 2, p99: 2 } },
    { backend: 'spider-chrome', scenario: 'js-catalog', conformance: counts(0, 1), warmup: counts(0), measurement: counts(0), throughput: { verifiedTasksPerSecond: 0 }, endToEndMs: { n: 0, p50: null, p95: null, p99: null } }
  ] });
  await write(`results/${bench}/manifest.json`, { schemaVersion: 1, id: bench, startedAt: '2026-10-07T21:25:08.994Z', config: { name: 'qualification' }, environment: env });
  const action = 'browser-actionability-2026-10-07T23-38-33-163Z-bbbbbb';
  await write(`results/${action}/summary.json`, { sourceChanges: ['src/a.ts'], cells: [
    { backend: 'stagehand', variant: 'overlay', policy: 'native', block: 0, status: 'startup-failed', reasons: ['adapter exited before ready'], taskMs: null },
    { backend: 'playwright', variant: 'overlay', policy: 'native', block: 0, status: 'verified', reasons: [], taskMs: 300 }
  ] });
  await write(`results/${action}/manifest.json`, { id: action, environment: env });
  const suite = 'qualification-integrated-2026-10-07T23-16-32-138Z-cccccc';
  await write(`results/${suite}/interrupted.json`, { reason: 'CPU counter units require calibration' });
  await write(`results/${suite}/manifest.json`, { id: suite, environment: env });
  await write('results/report-preview.png', 'not a run');
  await write('reports/evidence/catalog.json', { files: [
    { path: `evidence/${bench}/summary.json`, sha256: sha(benchSummary) },
    { path: `evidence/${suite}/manifest.json`, sha256: sha('tampered') }
  ] });
  await write(`reports/evidence/${bench}/summary.json`, benchSummary);
  await write(`reports/evidence/${suite}/manifest.json`, JSON.stringify({ id: suite, environment: env }));
  return { root, bench, action, suite };
}

async function treeHashes(directory: string): Promise<Record<string, string>> {
  const result: Record<string, string> = {};
  for (const entry of await readdir(directory, { withFileTypes: true, recursive: true })) {
    if (entry.isFile()) { const path = join(entry.parentPath, entry.name); result[path.slice(directory.length)] = sha(await readFile(path, 'utf8')); }
  }
  return result;
}

test('legacy runs of different shapes are re-expressed as valid run.v2 documents capped at local-dev', async () => {
  const lab = await legacyLab();
  const runs = await importLegacy({ root: lab.root });
  assert.deepEqual(runs.map(run => run.runId), [lab.bench, lab.suite, lab.action], 'ordered by start time');
  for (const run of runs) {
    assert.deepEqual(validateDocument('run.v2', run), [], run.runId);
    assert.equal(run.host.classGranted, 'local-dev');
    assert.equal(run.provenance.git.available, false);
  }
  const [bench, suite, action] = runs;
  assert.equal(bench!.status, 'complete');
  assert.equal(bench!.totals.verified, 7);
  assert.equal(bench!.cells.find(cell => cell.factors.backend === 'spider-chrome')!.status, 'failed');
  assert.equal(action!.status, 'invalid-provenance');
  const crashed = action!.cells.find(cell => cell.factors.backend === 'stagehand')!;
  assert.equal(crashed.status, 'crashed');
  assert.equal(crashed.errorCode, 'E_STARTUP');
  assert.equal(action!.legacy!.errorCodeInferred, true);
  assert.equal(suite!.status, 'interrupted');
  assert.equal(suite!.cells.length, 0);
});

test('integrity is checked against the evidence catalogs and a tampered copy is reported', async () => {
  const lab = await legacyLab();
  const runs = await importLegacy({ root: lab.root });
  const byId = new Map(runs.map(run => [run.runId, run]));
  assert.equal(byId.get(lab.bench)!.provenance.integrity.status, 'verified');
  assert.equal(byId.get(lab.action)!.provenance.integrity.status, 'uncatalogued');
  assert.equal(byId.get(lab.suite)!.provenance.integrity.status, 'mismatch');
});

test('import is idempotent and never modifies the original results', async () => {
  const lab = await legacyLab();
  const before = await treeHashes(join(lab.root, 'results'));
  const first = join(lab.root, 'out-1'), second = join(lab.root, 'out-2');
  await writeImported(await importLegacy({ root: lab.root }), first);
  await writeImported(await importLegacy({ root: lab.root }), second);
  assert.deepEqual(await treeHashes(first), await treeHashes(second));
  assert.deepEqual(await treeHashes(join(lab.root, 'results')), before);
});
