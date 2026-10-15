import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { runCi, defaultSteps } from '../tools/ci.ts';

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

test('a failed test step names each failing test with the first lines of its error', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'goodfetch-ci-'));
  await writeFile(join(dir, 'assertion.test.ts'), `import { test } from 'node:test'; import assert from 'node:assert/strict';
test('passes', () => {});
test('waits too little', () => { assert.equal(1, 2, 'deadline missed'); });\n`);
  await writeFile(join(dir, 'crash.test.ts'), `throw new TypeError('cannot load the fixture');\n`);
  const [tests] = defaultSteps(join(dir, 'failures.txt')).filter(step => step.failureLog);
  const result = await runCi([{ ...tests!, args: [...tests!.args.slice(0, -1), join(dir, '*.test.ts')] }], { quiet: true });
  const failures = result.steps[0]!.failures.join('\n');
  assert.equal(result.ok, false);
  assert.match(failures, /assertion\.test\.ts:3 waits too little\n {4}deadline missed\n {4}1 !== 2/);
  assert.match(failures, /crash\.test\.ts:1 \(file failed\)\n {4}TypeError: cannot load the fixture/);
  assert.doesNotMatch(failures, /passes/);
});
