// Impit (browser-impersonating HTTP client) on the same crawl as node-http: one GET per page, no retries.
import { Impit } from 'impit';
import { serve, type Adapter, type AdapterOutput, type Task } from '../../src/worker-server.ts';

const adapter: Adapter = {
  info() { return { backend: 'impit-http', protocol: 1, capabilities: ['http-catalog'], runtime: process.version, maxConcurrency: 128, lifecycle: 'client per task, Chrome emulation, no retries', versions: { impit: '0.14.5' } }; },
  async prepare() {},
  async run(task: Task): Promise<AdapterOutput> {
    if (task.scenario !== 'http-catalog') return { status: 'unsupported', reason: `unsupported ${task.scenario}` };
    const impit = new Impit({ browser: 'chrome', timeout: task.timeoutMs, ignoreTlsErrors: task.insecureTls === true });
    const start = performance.now(); const records: { id: string; price: number }[] = []; let url: string | null = task.url; let visited = 0;
    while (url) {
      if (visited++ > task.pages) throw new Error('pagination exceeded contract');
      const response = await impit.fetch(url); if (!response.ok) throw new Error(`HTTP ${response.status}`);
      const html = await response.text();
      for (const match of html.matchAll(/<article data-id="([^"]+)" data-price="(\d+)"/g)) records.push({ id: match[1]!, price: Number(match[2]) });
      const next = html.match(/class="next" href="([^"]+)"/); url = next ? new URL(next[1]!, url).href : null;
    }
    return { status: 'ok', records, metrics: { taskMs: performance.now() - start, pages: visited } };
  },
  async close() {}
};
serve(adapter);
