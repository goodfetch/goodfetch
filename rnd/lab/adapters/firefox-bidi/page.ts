// One browsing context of stock Firefox driven over WebDriver BiDi, as the lab's tasks drive a page. Every action goes
// through the Goodfetch Guard (guard.ts) and trusted input (`input.performActions`, `input.setFiles`); every read runs
// in the Guard's sandbox. Nothing is called in the page's main world.
import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import type { BrowserPage } from '../node/browser-tasks.ts';
import { gestureEvents } from '../browser-scripts/scripts.ts';
import { Bidi, BidiError, fromRemote } from './bidi.ts';
import { act, callInSandbox, isWorldGone, sandboxWorld, SANDBOX, type Acted, type Kind, type Point } from './guard.ts';

// The events a hover's arm watches: the pointer's arrival on the target. Not `pointerenter` or `mouseenter`, which
// also fire on each ancestor the pointer enters, as events of their own whose path does not hold the target.
const hoverEvents = ['pointerover', 'mouseover', 'pointermove', 'mousemove'] as const;
// WebDriver's key for Enter.
const ENTER = '';
const DELETE = '';

type PointerAction = { type: 'pointerMove'; x: number; y: number } | { type: 'pointerDown' | 'pointerUp'; button: number } | { type: 'pause'; duration: number };

export class BidiPage implements BrowserPage {
  readonly ownGuard = true;
  // Each guarded action this page took, in order, with the commands it sent.
  readonly acted: { kind: string; acted: Acted }[];
  private readonly bidi: Bidi;
  private readonly context: string;
  private readonly timeoutMs: number;
  constructor(bidi: Bidi, context: string, timeoutMs: number, acted: { kind: string; acted: Acted }[] = []) {
    this.bidi = bidi; this.context = context; this.timeoutMs = timeoutMs; this.acted = acted;
  }

  async goto(url: string): Promise<void> { await this.bidi.send('browsingContext.navigate', { context: this.context, url, wait: 'interactive' }); }

  // Evaluates `script` in the Guard's sandbox, awaiting its promise; a document replaced meanwhile is read again.
  async evaluate<T>(script: string): Promise<T> {
    const end = performance.now() + this.timeoutMs;
    for (;;) {
      try {
        const answer = await this.bidi.send('script.evaluate', { expression: script, awaitPromise: true, resultOwnership: 'none', target: { context: this.context, sandbox: SANDBOX } });
        if (answer.type === 'exception') throw new BidiError('script.evaluate', 'javascript error', (answer.exceptionDetails as { text?: string } | undefined)?.text ?? 'exception');
        return fromRemote(answer.result as Record<string, unknown>) as T;
      } catch (error) { if (!isWorldGone(error) || performance.now() > end) throw error; }
    }
  }

  click(selector: string): Promise<void> { return this.pointer('click', selector, gestureEvents.click, point => this.buttons(point, 0, 1)); }
  dblclick(selector: string): Promise<void> { return this.pointer('dblclick', selector, gestureEvents.dblclick, point => this.buttons(point, 0, 2)); }
  rightClick(selector: string): Promise<void> { return this.pointer('contextmenu', selector, gestureEvents.contextmenu, point => this.buttons(point, 2, 1)); }
  hover(selector: string): Promise<void> { return this.pointer('hover', selector, hoverEvents, point => this.mouse([{ type: 'pointerMove', ...point }])); }

  // The drag is checked and armed on its source; its drop point is read in the sandbox before.
  async drag(from: string, to: string): Promise<void> {
    const drop = await callInSandbox(this.bidi, this.context, '(selector)=>{const box=document.querySelector(selector).getBoundingClientRect();return {x:box.left+box.width/2,y:box.top+box.height/2}}', to) as Point;
    await this.pointer('drag', from, gestureEvents.drag, start => {
      const steps = Array.from({ length: 5 }, (_, index) => ({ type: 'pointerMove' as const, x: start.x + (drop.x - start.x) * (index + 1) / 5, y: start.y + (drop.y - start.y) * (index + 1) / 5 }));
      return this.mouse([{ type: 'pointerMove', ...start }, { type: 'pointerDown', button: 0 }, ...steps, { type: 'pointerUp', button: 0 }]);
    });
  }

  wheel(selector: string, deltaY: number): Promise<void> {
    return this.pointer('wheel', selector, gestureEvents.wheel, point => this.input([{ type: 'wheel', id: 'wheel', actions: [{ type: 'scroll', x: Math.round(point.x), y: Math.round(point.y), deltaX: 0, deltaY }] }]));
  }

  press(selector: string, key: 'Enter'): Promise<void> {
    const value = key === 'Enter' ? ENTER : key;
    return this.text('press', selector, 'text', () => this.keys([value]));
  }

  // The Guard focuses the field and selects its text; the text then arrives as typed, one grapheme per key.
  fill(selector: string, value: string): Promise<void> {
    const graphemes = [...new Intl.Segmenter().segment(value)].map(part => part.segment);
    return this.text('fill', selector, 'fill', () => this.keys(graphemes.length ? graphemes : [DELETE]));
  }

  select(): Promise<void> { return Promise.reject(new Error('select is not in the matrix')); }

  async close(): Promise<void> { await this.bidi.send('browsingContext.close', { context: this.context }); }

  async goBack(): Promise<void> { await this.bidi.send('browsingContext.traverseHistory', { context: this.context, delta: -1 }); }

  // The popup is the top-level context the guarded click creates, handed over once its document loaded.
  async withPopup(trigger: string, task: (popup: BrowserPage) => Promise<void>): Promise<void> {
    const created = this.bidi.next('browsingContext.contextCreated', event => event.parent === null && event.context !== this.context, this.timeoutMs);
    const loaded = new Set<string>(); const stop = this.bidi.on('browsingContext.domContentLoaded', event => { if (String(event.url).endsWith('/popup')) loaded.add(String(event.context)); });
    try {
      await this.click(trigger);
      const popup = String((await created).context);
      if (!loaded.has(popup)) await this.bidi.next('browsingContext.domContentLoaded', event => event.context === popup && String(event.url).endsWith('/popup'), this.timeoutMs);
      const page = new BidiPage(this.bidi, popup, this.timeoutMs, this.acted);
      try { await task(page); } finally { await page.close(); }
    } finally { stop(); }
  }

  // The frame is the child context of the element's content window, read in the sandbox.
  async withFrame(selector: string, task: (frame: BrowserPage) => Promise<void>): Promise<void> {
    const answer = await this.bidi.send('script.callFunction', { functionDeclaration: '(selector)=>document.querySelector(selector).contentWindow', arguments: [{ type: 'string', value: selector }], awaitPromise: false, resultOwnership: 'none', target: { context: this.context, sandbox: SANDBOX } });
    const window = answer.result as { type?: string; value?: { context?: string } } | undefined;
    if (window?.type !== 'window' || typeof window.value?.context !== 'string') throw new Error(`no frame for ${selector}: ${JSON.stringify(answer)}`);
    await task(new BidiPage(this.bidi, window.value.context, this.timeoutMs, this.acted));
  }

  // The Guard checks the field (unique, visible, enabled), then the files are set on the node BiDi located.
  async upload(selector: string, path: string): Promise<void> {
    await this.text('upload', selector, 'text', async () => {
      const located = await this.bidi.send('browsingContext.locateNodes', { context: this.context, locator: { type: 'css', value: selector }, maxNodeCount: 2 });
      const nodes = located.nodes as { sharedId?: string }[];
      if (nodes.length !== 1 || !nodes[0]?.sharedId) throw new Error(`${selector} located ${nodes.length} nodes`);
      await this.bidi.send('input.setFiles', { context: this.context, element: { sharedId: nodes[0].sharedId }, files: [path] });
    });
  }

  // A guarded click on the link, the download allowed into a fresh folder, read once BiDi reports it complete.
  async download(selector: string): Promise<string> {
    const folder = await mkdtemp(join(tmpdir(), 'goodfetch-bidi-download-'));
    await this.bidi.send('browser.setDownloadBehavior', { downloadBehavior: { type: 'allowed', destinationFolder: folder }, userContexts: ['default'] });
    const ended = this.bidi.next('browsingContext.downloadEnd', event => event.context === this.context, this.timeoutMs * 3);
    await this.click(selector);
    const end = await ended as { status?: string; filepath?: string | null };
    if (end.status !== 'complete' || !end.filepath) throw new Error(`download ended ${end.status}`);
    return readFile(end.filepath, 'utf8');
  }

  private async pointer(kind: string, selector: string, events: readonly string[], perform: (point: Point) => Promise<void>): Promise<void> {
    this.acted.push({ kind, acted: await act(sandboxWorld(this.bidi, this.context), { selector, kind: 'pointer', events, timeoutMs: this.timeoutMs }, perform) });
  }

  private async text(kind: string, selector: string, guardKind: Kind, perform: () => Promise<void>): Promise<void> {
    this.acted.push({ kind, acted: await act(sandboxWorld(this.bidi, this.context), { selector, kind: guardKind, events: [], timeoutMs: this.timeoutMs }, perform) });
  }

  private buttons(point: Point, button: number, count: number): Promise<void> {
    const press: PointerAction[] = Array.from({ length: count }, () => [{ type: 'pointerDown' as const, button }, { type: 'pointerUp' as const, button }]).flat();
    return this.mouse([{ type: 'pointerMove', ...point }, ...press]);
  }

  private mouse(actions: PointerAction[]): Promise<void> { return this.input([{ type: 'pointer', id: 'mouse', parameters: { pointerType: 'mouse' }, actions }]); }

  private keys(values: string[]): Promise<void> {
    return this.input([{ type: 'key', id: 'keyboard', actions: values.flatMap(value => [{ type: 'keyDown', value }, { type: 'keyUp', value }]) }]);
  }

  // One burst of trusted input, one command.
  private async input(actions: Record<string, unknown>[]): Promise<void> { await this.bidi.send('input.performActions', { context: this.context, actions }); }
}
