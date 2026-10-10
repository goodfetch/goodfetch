import { join } from 'node:path';
import { startFixtures, type RunSpec } from '../../fixtures/server.ts';
import { WorkerClient } from '../worker-client.ts';
import { labRoot } from '../registry.ts';

export type Verdict = { status: 'verified' | 'failed'; reasons: string[] };
export type Oracle = {
  origin: string; mode: 'process' | 'in-process';
  createRun(spec: RunSpec): Promise<{ id: string; url: string }>;
  verify(id: string, output: unknown): Promise<Verdict>;
  snapshot(id: string): Promise<Record<string, unknown>>;
  close(): Promise<void>;
};

// `process` keeps fixtures and the private oracle out of the measured event loop; `in-process` is for unit tests.
export async function startOracle(mode: 'process' | 'in-process' = 'process'): Promise<Oracle> {
  if (mode === 'in-process') {
    const fixtures = await startFixtures();
    return { origin: fixtures.origin, mode, createRun: spec => fixtures.createRun(spec), verify: async (id, output) => fixtures.verify(id, output as never) as Promise<Verdict>,
      snapshot: async id => fixtures.snapshot(id) as Record<string, unknown>, close: () => fixtures.close() };
  }
  const worker = new WorkerClient({ command: process.execPath, args: [join(labRoot, 'fixtures/worker.ts')], cwd: labRoot, timeoutMs: 30_000, env: process.env });
  const { origin } = await worker.call('hello', {}) as { origin: string };
  return { origin, mode, createRun: async spec => await worker.call('createRun', spec) as { id: string; url: string },
    verify: async (id, output) => await worker.call('verify', { id, output }) as Verdict,
    snapshot: async id => await worker.call('snapshot', { id }) as Record<string, unknown>, close: () => worker.close() };
}
