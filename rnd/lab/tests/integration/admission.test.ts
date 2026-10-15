import { test } from 'node:test';
import assert from 'node:assert/strict';
import { existsSync, mkdtempSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runAdmissionCell, admissionBinary, type AdmissionConfig } from '../../src/admission-campaign.ts';
import { labRoot } from '../../src/registry.ts';

const flood = { ...JSON.parse(readFileSync(join(labRoot, 'campaigns/v2/admission/durable-scheduler.json'), 'utf8')).config, durationMs: 1500 } as AdmissionConfig;
const run = (policy: 'fifo-global' | 'fair-domain') => runAdmissionCell(flood, { policy, scenario: 'flood', fault: 'none' }, 0, mkdtempSync(join(tmpdir(), 'goodfetch-admission-'))) as Promise<{ status: string; reasons: string[]; rejected: number; lightAcceptance: number; peakInflight: Record<string, number> }>;

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

test('killed while overloaded, the runtime recovers every journaled job, a resubmitted job is acknowledged once, and nothing is lost', { skip: !existsSync(admissionBinary()) && 'run pnpm workflow-build' }, async () => {
  const crash = { ...JSON.parse(readFileSync(join(labRoot, 'campaigns/v2/admission/durable-admission-recovery.json'), 'utf8')).config, durationMs: 1500, crashAtMs: 700 } as AdmissionConfig;
  const cell = await runAdmissionCell(crash, { policy: 'fair-domain', scenario: crash.scenario, fault: 'crash' }, 0, mkdtempSync(join(tmpdir(), 'goodfetch-admission-'))) as { status: string; reasons: string[]; recovered: number; resubmitted: number; duplicateAcks: number };
  assert.equal(cell.status, 'verified', cell.reasons.join('\n'));
  assert.ok(cell.recovered > 0, 'jobs were waiting when the runtime died');
  assert.ok(cell.duplicateAcks >= 1, 'the submission journaled before the crash is acknowledged as a duplicate on resubmission');
});
