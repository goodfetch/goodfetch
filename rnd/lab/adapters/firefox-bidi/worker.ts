// The lab worker for stock Firefox over WebDriver BiDi with the Goodfetch Guard in a sandbox (issue 2460): the
// actionability, primitives and navigation tasks of the cross-engine matrix, with the guarded policy only, since every
// action of this route goes through the Guard. Each task's metrics carry what the session's own log of sent commands
// shows: the commands of each click, and the script calls that targeted the page's main world (none expected).
import { serve, type Adapter, type AdapterOutput, type Task } from '../../src/worker-server.ts';
import { runBrowserTask } from '../node/browser-tasks.ts';
import { Bidi } from './bidi.ts';
import { BidiPage } from './page.ts';

const capabilities = ['actionability', 'primitives', 'navigation'];
let bidi: Bidi | undefined;

const adapter: Adapter = {
  info() { return { backend: 'firefox-bidi', protocol: 1, capabilities, runtime: process.version, maxConcurrency: 1, lifecycle: 'browser per worker; tab per task', versions: { firefox: bidi?.version ?? 'not started' } }; },
  async prepare(input) {
    if (!input.executablePath) throw new Error('firefox-bidi needs the installed Firefox as executablePath');
    bidi = await Bidi.launch(input.executablePath);
    await bidi.send('session.subscribe', { events: ['browsingContext.contextCreated', 'browsingContext.domContentLoaded', 'browsingContext.downloadEnd'] });
  },
  async run(task: Task): Promise<AdapterOutput> {
    if (!bidi) throw new Error('adapter not prepared');
    if (!capabilities.includes(task.scenario)) return { status: 'unsupported', reason: `unsupported ${task.scenario}` };
    if (task.actionPolicy !== 'guarded') return { status: 'unsupported', reason: 'the BiDi route acts only through the Guard' };
    const created = await bidi.send('browsingContext.create', { type: 'tab' });
    const context = String(created.context);
    await bidi.send('browsingContext.setViewport', { context, viewport: { width: 1280, height: 720 } });
    const firstCommand = bidi.sent.length;
    const page = new BidiPage(bidi, context, task.timeoutMs);
    try {
      const output = await runBrowserTask(page, task);
      return { ...output, metrics: { ...output.metrics, ...logMetrics(bidi, firstCommand, page) } };
    } finally { await page.close().catch(() => {}); }
  },
  async close() { await bidi?.close(); bidi = undefined; }
};

// What the session's log shows for one task: its commands, its script calls outside the sandbox, and the first click's
// commands by kind (its checks, its input and its disarm).
function logMetrics(session: Bidi, firstCommand: number, page: BidiPage): Record<string, number> {
  const sent = session.sent.slice(firstCommand);
  const scripts = sent.filter(command => command.method.startsWith('script.'));
  const click = page.acted.find(entry => entry.kind === 'click')?.acted;
  return {
    bidiCommands: sent.length, scriptCalls: scripts.length, mainWorldCalls: scripts.filter(command => command.sandbox === undefined).length,
    guardedActions: page.acted.length,
    ...(click ? { clickCommands: click.commands, clickAttempts: click.attempts, clickChecksAndDisarms: click.methods.filter(method => method === 'script.callFunction').length, clickInputs: click.methods.filter(method => method === 'input.performActions').length } : {})
  };
}

serve(adapter);
