// Direct CDP reference driver in Node: same page scripts and command sequence as the Rust driver,
// so a comparison between the two isolates the language and runtime from the protocol.
import { serve, type Adapter, type AdapterOutput, type Task } from '../../src/worker-server.ts';
import { launchBrowser, type HostedBrowser } from '../../src/core/browser-host.ts';
import { runBrowserTask } from '../node/browser-tasks.ts';
import { CdpConnection, CdpPage } from './cdp.ts';

let connection: CdpConnection | undefined, browser: HostedBrowser | undefined;
const adapter: Adapter<Task, { executablePath?: string; cdpEndpoint?: string }> = {
  info() { return { backend: 'node-cdp', protocol: 1, capabilities: ['http-catalog', 'js-catalog', 'form', 'clicks', 'actionability', 'shadow', 'primitives', 'navigation'], runtime: process.version, maxConcurrency: 16, lifecycle: 'harness or own browser per block; target per task', versions: { cdp: 'direct' } }; },
  async prepare(input) {
    if (input.cdpEndpoint) { connection = await CdpConnection.connect(input.cdpEndpoint); return; }
    if (!input.executablePath) throw new Error('executablePath or cdpEndpoint required');
    browser = await launchBrowser(input.executablePath); connection = await CdpConnection.connect(browser.wsEndpoint);
  },
  async run(task): Promise<AdapterOutput> {
    if (!connection) throw new Error('adapter not prepared');
    const page = await CdpPage.open(connection, task.timeoutMs);
    let output: AdapterOutput;
    try { output = await runBrowserTask(page, task); } finally { await page.close(); }
    return { ...output, metrics: { ...output.metrics, cdpCommands: page.commands } };
  },
  async close() { connection?.close(); await browser?.close(); }
};
serve(adapter);
