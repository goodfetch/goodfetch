// Mozilla Readability (the Firefox reader mode) on linkedom, for main-content extraction quality.
import { readFileSync } from 'node:fs';
import { gunzipSync } from 'node:zlib';
import { Readability } from '@mozilla/readability';
import { parseHTML } from 'linkedom';
import { serve, type Adapter } from '../../src/worker-server.ts';

const adapter: Adapter<{ path: string; url?: string }> = {
  info() { return { backend: 'readability', protocol: 1, capabilities: ['extract'], runtime: process.version, maxConcurrency: 1, versions: { '@mozilla/readability': '0.6.0', linkedom: '0.18.13' } }; },
  async prepare() {},
  async run(task) {
    const raw = readFileSync(task.path); const html = (task.path.endsWith('.gz') ? gunzipSync(raw) : raw).toString('utf8');
    const began = performance.now(); const { document } = parseHTML(html);
    const text = new Readability(document as unknown as Document).parse()?.textContent ?? '';
    return { status: 'ok', text, metrics: { extractMs: performance.now() - began, htmlBytes: html.length } };
  },
  async close() {}
};
serve(adapter);
