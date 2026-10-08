import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runAdmissionCell, admissionBinary, type AdmissionConfig } from '../../src/admission-campaign.ts';
import { labRoot } from '../../src/registry.ts';

const flood = { ...JSON.parse(readFileSync(join(labRoot, 'campaigns/v2/admission/durable-scheduler.json'), 'utf8')).config, durationMs: 1500 } as AdmissionConfig;
const run = (policy: 'fifo-global' | 'fair-domain') => runAdmissionCell(flood, { policy, scenario: 'flood' }, 0, mkdtempSync(join(tmpdir(), 'automan-admission-'))) as Promise<{ status: string; reasons: string[]; rejected: number; lightAcceptance: number; peakInflight: Record<string, number> }>;

test('under a heavy domain flood, both policies lose nothing and keep every domain within its limit; only per-domain queues keep light domains admitted', { skip: !existsSync(admissionBinary()) && 'run pnpm workflow-build' }, async () => {
  const [global, fair] = [await run('fifo-global'), await run('fair-domain')];
  for (const cell of [global, fair]) {
    assert.equal(cell.status, 'verified', cell.reasons.join('\n'));
    assert.ok(cell.rejected > 0, 'the overload is rejected explicitly');
    assert.ok(Object.values(cell.peakInflight).every(peak => peak <= flood.domainLimit));
  }
  assert.equal(fair.lightAcceptance, 1);
  assert.ok(global.lightAcceptance < 0.9, `light acceptance under a global queue: ${global.lightAcceptance}`);
});
