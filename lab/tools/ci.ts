// Local CI: typecheck, unit and contract tests, registry and campaign validation, stopping at the first failure.
// Portable checks only; performance comparisons run as campaigns on a controlled host, never here.
import { spawn } from 'node:child_process';
import { labRoot } from '../src/registry.ts';

export type CiStep = { name: string; command: string; args: string[] };
export const defaultSteps = (): CiStep[] => [
  { name: 'typecheck', command: process.execPath, args: ['node_modules/typescript/bin/tsc', '--noEmit'] },
  { name: 'unit and contract tests', command: process.execPath, args: ['--test', 'tests/*.test.ts'] },
  { name: 'registry and campaigns', command: process.execPath, args: ['tools/lab.ts', 'validate'] }
];

export async function runCi(steps: CiStep[], { quiet = false }: { quiet?: boolean } = {}) {
  const results: { name: string; code: number; durationMs: number }[] = [];
  for (const step of steps) {
    const began = performance.now();
    const code = await new Promise<number>(resolve => {
      const child = spawn(step.command, step.args, { cwd: labRoot, stdio: quiet ? 'ignore' : 'inherit' });
      child.on('exit', (exit, signal) => resolve(exit ?? (signal ? 128 : 1)));
      child.on('error', () => resolve(127));
    });
    results.push({ name: step.name, code, durationMs: performance.now() - began });
    if (!quiet) process.stderr.write(`${code === 0 ? 'ok  ' : 'FAIL'} ${step.name}\n`);
    if (code !== 0) break;
  }
  return { ok: results.length === steps.length && results.every(result => result.code === 0), steps: results };
}
