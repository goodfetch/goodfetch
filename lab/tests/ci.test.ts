import { test } from 'node:test';
import assert from 'node:assert/strict';
import { runCi } from '../tools/ci.ts';

const step = (name: string, code: number) => ({ name, command: process.execPath, args: ['-e', `process.exit(${code})`] });

test('the local CI stops at the first failing step and reports it', async () => {
  const result = await runCi([step('check', 0), step('test', 3), step('validate', 0)], { quiet: true });
  assert.equal(result.ok, false);
  assert.deepEqual(result.steps.map(s => [s.name, s.code]), [['check', 0], ['test', 3]]);
});

test('the local CI succeeds only when every step succeeds', async () => {
  const result = await runCi([step('check', 0), step('test', 0)], { quiet: true });
  assert.equal(result.ok, true);
  assert.equal(result.steps.length, 2);
});
