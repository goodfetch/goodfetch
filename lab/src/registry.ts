import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import type { WorkerCommand } from './worker-client.ts';
export const labRoot = fileURLToPath(new URL('../', import.meta.url));
// The shared SQLite build is a .dylib on macOS and a .so on Linux; artifacts and hashes use the platform's file name.
export const sqliteLibrary = process.platform === 'darwin' ? 'libautoman_sqlite.dylib' : 'libautoman_sqlite.so';
export const backendNames = ['node-http', 'crawlee-http', 'crawlee-http-pooled', 'crawlee-playwright', 'playwright', 'playwright-firefox', 'playwright-webkit', 'puppeteer', 'stagehand', 'stagehand-batch', 'crawlee-stagehand', 'spider-http', 'spider-chrome', 'spider-smart', 'node-cdp', 'rust-cdp', 'reqwest-http', 'wreq-http', 'impit-http', 'rs-trafilatura', 'dom-smoothie', 'scraper-text', 'htmd', 'readability'] as const;
export type BackendName = typeof backendNames[number];
export function commandFor(backend: BackendName, timeoutMs: number, executablePath?: string): WorkerCommand {
  if (backend.startsWith('spider')) return { command: join(labRoot, 'adapters/rust/target', backend === 'spider-http' ? 'http' : backend === 'spider-chrome' ? 'chrome' : 'smart', 'release/automan-lab-spider'), args: [backend], cwd: labRoot, timeoutMs, env:{...process.env,...(executablePath?{CHROME_BIN:executablePath}:{})} };
  if (['rs-trafilatura', 'dom-smoothie', 'scraper-text', 'htmd'].includes(backend)) return { command: join(labRoot, 'adapters/extract-rust/target/release/automan-lab-extract'), args: [backend], cwd: labRoot, timeoutMs, env: { ...process.env } };
  if (backend === 'reqwest-http' || backend === 'wreq-http') return { command: join(labRoot, 'adapters/http-rust/target', backend === 'reqwest-http' ? 'reqwest' : 'wreq', 'release/automan-lab-http'), args: [backend], cwd: labRoot, timeoutMs, env: { ...process.env } };
  if (backend === 'rust-cdp') return { command: join(labRoot, 'adapters/cdp-rust/target/release/automan-lab-cdp'), args: [backend], cwd: labRoot, timeoutMs, env: { ...process.env } };
  return { command: process.execPath, args: [join(labRoot, backend === 'node-cdp' ? 'adapters/cdp-node/worker.ts' : backend === 'impit-http' ? 'adapters/node/impit-worker.ts' : backend === 'readability' ? 'adapters/node/readability-worker.ts' : backend === 'crawlee-stagehand' ? 'adapters/stagehand-v3/worker.ts' : backend === 'crawlee-http-pooled' ? 'adapters/node/pooled-worker.ts' : 'adapters/node/worker.ts'), backend], cwd: labRoot, timeoutMs, env: { ...process.env, PLAYWRIGHT_BROWSERS_PATH: join(labRoot, '.browsers'), CRAWLEE_STORAGE_DIR: join(labRoot, '.state'), DO_NOT_TRACK: '1' } };
}
