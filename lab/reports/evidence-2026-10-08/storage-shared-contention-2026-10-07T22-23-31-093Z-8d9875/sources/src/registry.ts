import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import type { WorkerCommand } from './worker-client.ts';
export const labRoot = fileURLToPath(new URL('../', import.meta.url));
export const backendNames = ['node-http', 'crawlee-http', 'crawlee-http-pooled', 'crawlee-playwright', 'playwright', 'puppeteer', 'stagehand', 'stagehand-batch', 'crawlee-stagehand', 'spider-http', 'spider-chrome', 'spider-smart'] as const;
export type BackendName = typeof backendNames[number];
export function commandFor(backend: BackendName, timeoutMs: number, executablePath?: string): WorkerCommand {
  if (backend.startsWith('spider')) return { command: join(labRoot, 'adapters/rust/target', backend === 'spider-http' ? 'http' : backend === 'spider-chrome' ? 'chrome' : 'smart', 'release/automan-lab-spider'), args: [backend], cwd: labRoot, timeoutMs, env:{...process.env,...(executablePath?{CHROME_BIN:executablePath}:{})} };
  return { command: process.execPath, args: [join(labRoot, backend === 'crawlee-stagehand' ? 'adapters/stagehand-v3/worker.ts' : backend === 'crawlee-http-pooled' ? 'adapters/node/pooled-worker.ts' : 'adapters/node/worker.ts'), backend], cwd: labRoot, timeoutMs, env: { ...process.env, PLAYWRIGHT_BROWSERS_PATH: join(labRoot, '.browsers'), CRAWLEE_STORAGE_DIR: join(labRoot, '.state'), DO_NOT_TRACK: '1' } };
}
