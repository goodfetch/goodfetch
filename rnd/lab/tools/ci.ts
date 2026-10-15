// Local CI: typecheck, unit and contract tests, timing tests, registry and campaign validation, stopping at the first failure.
// Portable checks only; performance comparisons run as campaigns on a controlled host, never here.
import { spawn } from 'node:child_process';
import { readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { labRoot } from '../src/registry.ts';

// failureLog is a file the step writes a failure summary to; a failed step prints it, so a flake is named from one run.
export type CiStep = { name: string; command: string; args: string[]; failureLog?: string };
// Timer-precision tests run alone, after the parallel suite, whose startup burst would delay their timers.
export const defaultSteps = (failureLog = join(tmpdir(), `goodfetch-ci-${process.pid}-test-failures.txt`)): CiStep[] => {
  const nodeTest = (...args: string[]) => ['--test', ...args, '--test-reporter=spec', '--test-reporter-destination=stdout', '--test-reporter=./tools/test-failures.ts', `--test-reporter-destination=${failureLog}`];
  return [
    { name: 'typecheck', command: process.execPath, args: ['node_modules/typescript/bin/tsc', '--noEmit'] },
    { name: 'unit and contract tests', command: process.execPath, failureLog, args: [...nodeTest(), 'tests/*.test.ts'] },
    { name: 'timing tests', command: process.execPath, failureLog, args: [...nodeTest('--test-concurrency=1'), 'tests/timing/*.test.ts'] },
    { name: 'registry and campaigns', command: process.execPath, args: ['tools/lab.ts', 'validate'] }
  ];
};

export async function runCi(steps: CiStep[], { quiet = false }: { quiet?: boolean } = {}) {
  // A step's own node --test must not report into an enclosing test runner, as when this module is itself under test.
  const { NODE_TEST_CONTEXT: _, ...env } = process.env;
  const results: { name: string; code: number; durationMs: number; failures: string[] }[] = [];
  for (const step of steps) {
    if (step.failureLog) await rm(step.failureLog, { force: true });
    const began = performance.now();
    const code = await new Promise<number>(resolve => {
      const child = spawn(step.command, step.args, { cwd: labRoot, env, stdio: quiet ? 'ignore' : 'inherit' });
      child.on('exit', (exit, signal) => resolve(exit ?? (signal ? 128 : 1)));
      child.on('error', () => resolve(127));
    });
    const failures = code !== 0 && step.failureLog ? (await readFile(step.failureLog, 'utf8').catch(() => '')).split('\n').filter(Boolean) : [];
    if (step.failureLog) await rm(step.failureLog, { force: true });
    results.push({ name: step.name, code, durationMs: performance.now() - began, failures });
    if (!quiet) process.stderr.write(`${code === 0 ? 'ok  ' : 'FAIL'} ${step.name}\n${failures.map(line => `  ${line}\n`).join('')}`);
    if (code !== 0) break;
  }
  return { ok: results.length === steps.length && results.every(result => result.code === 0), steps: results };
}
