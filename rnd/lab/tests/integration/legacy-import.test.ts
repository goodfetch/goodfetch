import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync } from 'node:fs';
import { join } from 'node:path';
import { importLegacy } from '../../tools/import-legacy.ts';
import { labRoot } from '../../src/registry.ts';

// Control on the real local results: the counts below are the ones published in lab/reports.
const published: Record<string, number> = {
  'runtime-native-qualified-2026-10-08T00-11-54-841Z-789d5e': 30,
  'runtime-recovery-qualified-2026-10-08T00-20-09-515Z-ad3e70': 42,
  'browser-actionability-2026-10-08T00-21-16-186Z-b21a28': 162,
  'sqlite-readers-2026-10-08T00-25-35-434Z-92d4f3': 108,
  'spider-http-transport-supported-2026-10-08T00-26-16-511Z-7ce401': 60,
  'runtime-http-backends-2026-10-07T23-29-49-072Z-2b2b75': 24
};

test('importing the real results reproduces the published verified counts with intact evidence', { skip: !existsSync(join(labRoot, 'results', Object.keys(published)[0]!)) && 'local results/ not present' }, async () => {
  const runs = new Map((await importLegacy()).map(run => [run.runId, run]));
  for (const [runId, verified] of Object.entries(published)) {
    const run = runs.get(runId);
    assert.ok(run, runId);
    assert.equal(run.totals.verified, verified, runId);
    assert.equal(run.provenance.integrity.status, 'verified', runId);
  }
  assert.equal([...runs.values()].filter(run => run.provenance.integrity.status === 'mismatch').length, 0);
});
