// The control for issue 2460: the same Guard scripts and the same loop (guard.ts) on Chromium over CDP, as the engine
// drives them (an isolated world from `Page.createIsolatedWorld`, `Runtime.callFunctionOn`, three
// `Input.dispatchMouseEvent`), on the actionability cells, with the same fixtures and private oracle. A cell that fails
// on both browsers fails on the Guard, not on the WebDriver BiDi route.
// Usage: node adapters/firefox-bidi/chromium-control.ts <chrome> [blocks] [variant...]
import { startFixtures, type RunSpec } from '../../fixtures/server.ts';
import { launchBrowser, type HostedBrowser } from '../../src/core/browser-host.ts';
import { condition, gestureEvents, waitScript } from '../browser-scripts/scripts.ts';
import { CdpConnection } from '../cdp-node/cdp.ts';
import { act, WorldGone, type GuardWorld } from './guard.ts';

type Variant = NonNullable<RunSpec['variant']>;
const variants: Variant[] = ['normal', 'overlay', 'delayed', 'hidden', 'moving', 'rerender', 'never-enabled', 'never-uncovered', 'ambiguous'];
const negatives: Variant[] = ['never-enabled', 'never-uncovered', 'ambiguous'];

async function runCell(executablePath: string, variant: Variant, seed: number, timeoutMs: number) {
  const fixtures = await startFixtures(); let browser: HostedBrowser | undefined; let connection: CdpConnection | undefined;
  try {
    browser = await launchBrowser(executablePath, 30_000, process.getuid?.() === 0 ? ['--no-sandbox'] : []);
    connection = await CdpConnection.connect(browser.wsEndpoint);
    // The lab's connection, with the log of every command sent that the Guard's loop counts.
    const sent: string[] = []; const open = connection;
    const cdp = { sent, send: (method: string, params: Record<string, unknown> = {}, sessionId?: string) => { sent.push(method); return open.send(method, params, sessionId); } };
    const { targetId } = await cdp.send('Target.createTarget', { url: 'about:blank' }) as { targetId: string };
    const { sessionId } = await cdp.send('Target.attachToTarget', { targetId, flatten: true }) as { sessionId: string };
    await cdp.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false }, sessionId);
    const fixture = await fixtures.createRun({ scenario: 'actionability', seed, pages: 1, variant });
    await cdp.send('Page.navigate', { url: fixture.url }, sessionId);
    const { frameTree } = await cdp.send('Page.getFrameTree', {}, sessionId) as { frameTree: { frame: { id: string } } };
    let context: number | undefined;
    const world: GuardWorld = {
      sent: () => cdp.sent,
      async call(functionDeclaration, argument) {
        context ??= (await cdp.send('Page.createIsolatedWorld', { frameId: frameTree.frame.id, worldName: 'goodfetch-guard' }, sessionId)).executionContextId as number;
        try {
          const answer = await cdp.send('Runtime.callFunctionOn', { functionDeclaration, executionContextId: context, arguments: argument === undefined ? [] : [{ value: argument }], awaitPromise: true, returnByValue: true }, sessionId) as { result: { value?: unknown }; exceptionDetails?: { text: string } };
          if (answer.exceptionDetails) throw new Error(answer.exceptionDetails.text);
          return answer.result.value;
        } catch (error) { if (/context/i.test(String(error))) { context = undefined; throw new WorldGone(String(error)); } throw error; }
      }
    };
    await world.call(`async()=>${waitScript(condition('body-ready'), timeoutMs)}`);
    let nativeRejected = 0;
    try {
      await act(world, { selector: '#target', kind: 'pointer', events: gestureEvents.click, timeoutMs }, async ({ x, y }) => {
        await cdp.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y }, sessionId);
        await cdp.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', clickCount: 1 }, sessionId);
        await cdp.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', clickCount: 1 }, sessionId);
      });
    } catch (error) { nativeRejected = 1; if (!negatives.includes(variant)) throw error; }
    if (!negatives.includes(variant)) await world.call(`async()=>${waitScript(condition('action-saved'), timeoutMs)}`);
    return { variant, ...await fixtures.verify(fixture.id, { status: 'ok', metrics: { nativeRejected } }), ...fixtures.snapshot(fixture.id) };
  } catch (error) { return { variant, status: 'failed', reasons: [String(error)] }; }
  finally { connection?.close(); await browser?.close(); await fixtures.close(); }
}

if (process.argv[1]?.endsWith('chromium-control.ts')) {
  const [executablePath, blocks = '3', ...chosen] = process.argv.slice(2);
  if (!executablePath) throw new Error('usage: chromium-control.ts <chrome> [blocks] [variant...]');
  const cells: Record<string, unknown>[] = [];
  for (let block = 0; block < Number(blocks); block++) for (const variant of chosen.length ? chosen as Variant[] : variants) {
    const cell = { block, ...await runCell(executablePath, variant, 4242 + block, 800) }; cells.push(cell);
    process.stderr.write(`${variant}/b${block}: ${String(cell.status)}${cell.status === 'verified' ? '' : ` ${JSON.stringify(cell)}`}\n`);
  }
  process.stdout.write(JSON.stringify({ held: cells.filter(cell => cell.status === 'verified').length, cells: cells.length }) + '\n');
}
