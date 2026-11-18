import { fileURLToPath } from 'node:url';
import { join } from 'node:path';
import type { WorkerCommand } from './worker-client.ts';
export const labRoot = fileURLToPath(new URL('../', import.meta.url));
// The shared SQLite build is a .dylib on macOS and a .so on Linux; artifacts and hashes use the platform's file name.
export const sqliteLibrary = process.platform === 'darwin' ? 'libgoodfetch_sqlite.dylib' : 'libgoodfetch_sqlite.so';
export const backendNames = ['node-http', 'crawlee-http', 'crawlee-http-pooled', 'crawlee-playwright', 'playwright', 'playwright-firefox', 'playwright-webkit', 'puppeteer', 'stagehand', 'stagehand-batch', 'crawlee-stagehand', 'spider-http', 'spider-chrome', 'spider-smart', 'node-cdp', 'firefox-bidi', 'rust-cdp', 'reqwest-http', 'wreq-http', 'impit-http', 'rs-trafilatura', 'dom-smoothie', 'scraper-text', 'htmd', 'readability', 'lol-html', 'lol-html-htmd', 'rs-trafilatura-md', 'dom-smoothie-md', 'rs-trafilatura-htmd', 'rs-trafilatura-html2md', 'rs-trafilatura-fast-html2md', 'rs-trafilatura-mdka', 'conv-htmd', 'conv-html2md', 'conv-fast-html2md', 'conv-mdka', 'conv-quick-html2md', 'scraper-fields', 'lol-html-fields'] as const;
export type BackendName = typeof backendNames[number];
// Every extractor, converter and field reader served by adapters/extract-rust (the mode is its first argument).
export const rustExtractors: readonly string[] = ['rs-trafilatura', 'dom-smoothie', 'scraper-text', 'htmd', 'lol-html', 'lol-html-htmd', 'rs-trafilatura-md', 'dom-smoothie-md', 'rs-trafilatura-htmd', 'rs-trafilatura-html2md', 'rs-trafilatura-fast-html2md', 'rs-trafilatura-mdka', 'conv-htmd', 'conv-html2md', 'conv-fast-html2md', 'conv-mdka', 'conv-quick-html2md', 'scraper-fields', 'lol-html-fields'];
export function commandFor(backend: BackendName, timeoutMs: number, executablePath?: string): WorkerCommand {
  if (backend.startsWith('spider')) return { command: join(labRoot, 'adapters/rust/target', backend === 'spider-http' ? 'http' : backend === 'spider-chrome' ? 'chrome' : 'smart', 'release/goodfetch-lab-spider'), args: [backend], cwd: labRoot, timeoutMs, env:{...process.env,...(executablePath?{CHROME_BIN:executablePath}:{})} };
  if (rustExtractors.includes(backend)) return { command: join(labRoot, 'adapters/extract-rust/target/release/goodfetch-lab-extract'), args: [backend], cwd: labRoot, timeoutMs, env: { ...process.env } };
  if (backend === 'reqwest-http' || backend === 'wreq-http') return { command: join(labRoot, 'adapters/http-rust/target', backend === 'reqwest-http' ? 'reqwest' : 'wreq', 'release/goodfetch-lab-http'), args: [backend], cwd: labRoot, timeoutMs, env: { ...process.env } };
  // Stock Firefox over WebDriver BiDi; executablePath is the installed Firefox, not the campaign's Chromium.
  if (backend === 'firefox-bidi') return { command: process.execPath, args: [join(labRoot, 'adapters/firefox-bidi/worker.ts')], cwd: labRoot, timeoutMs, env: { ...process.env } };
  if (backend === 'rust-cdp') return { command: join(labRoot, 'adapters/cdp-rust/target/release/goodfetch-lab-cdp'), args: [backend], cwd: labRoot, timeoutMs, env: { ...process.env } };
  return { command: process.execPath, args: [join(labRoot, backend === 'node-cdp' ? 'adapters/cdp-node/worker.ts' : backend === 'impit-http' ? 'adapters/node/impit-worker.ts' : backend === 'readability' ? 'adapters/node/readability-worker.ts' : backend === 'crawlee-stagehand' ? 'adapters/stagehand-v3/worker.ts' : backend === 'crawlee-http-pooled' ? 'adapters/node/pooled-worker.ts' : 'adapters/node/worker.ts'), backend], cwd: labRoot, timeoutMs, env: { ...process.env, PLAYWRIGHT_BROWSERS_PATH: join(labRoot, '.browsers'), CRAWLEE_STORAGE_DIR: join(labRoot, '.state'), DO_NOT_TRACK: '1' } };
}
