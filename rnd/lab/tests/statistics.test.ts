import { test } from 'node:test';
import assert from 'node:assert/strict';
import { summarize } from '../src/statistics.ts';
test('failed and unstarted attempts stay in denominators and are never fast successes', () => {
  const summary = summarize([
    { status: 'verified', elapsedMs: 10 }, { status: 'verified', elapsedMs: 30 },
    { status: 'failed', elapsedMs: 1000 }, { status: 'rejected', elapsedMs: null },
    { status: 'unsupported', elapsedMs: null }
  ]);
  assert.equal(summary.attempts, 5); assert.equal(summary.verified, 2);
  assert.equal(summary.successRate, 0.4); assert.equal(summary.successLatencyMs.p95, 30);
  assert.equal(summary.startedLatencyMs.p95, 1000); assert.equal(summary.rejected, 1);
});
