import { test } from 'node:test';
import assert from 'node:assert/strict';
import { schedule } from '../src/scheduler.ts';

test('open arrivals stop at the admission deadline and account for every unstarted task', async () => {
  const started: number[] = [], rejected: number[] = [];
  const result = await schedule({ mode: 'open', iterations: 4, concurrency: 1, ratePerSecond: 2, maxDurationMs: 20 },
    async index => { started.push(index); }, index => { rejected.push(index); });
  assert.deepEqual(started, [0]); assert.deepEqual(rejected, [1, 2, 3]);
  assert.ok(result.durationMs < 400, `deadline took ${result.durationMs} ms`);
});

test('open overload rejects arrivals without serializing the generator behind slow completions', async () => {
  let active = 0, peak = 0; const started: number[] = [], rejected: number[] = [];
  await schedule({ mode: 'open', iterations: 12, concurrency: 2, ratePerSecond: 1000, maxDurationMs: 500 }, async index => {
    started.push(index); peak = Math.max(peak, ++active);
    await new Promise(resolve => setTimeout(resolve, 80)); active--;
  }, index => { rejected.push(index); });
  assert.equal(peak, 2); assert.equal(started.length + rejected.length, 12);
  assert.ok(rejected.length >= 8); assert.equal(new Set([...started, ...rejected]).size, 12);
});
