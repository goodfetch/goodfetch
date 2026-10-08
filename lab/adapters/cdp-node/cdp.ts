// Minimal Chrome DevTools Protocol client over the runtime's WebSocket. The command sequence per page
// operation is the contract shared with the Rust driver (adapters/cdp-rust/src/cdp.rs).
import { mkdtemp, readFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pointScript, focusScript, selectScript, frameOffsetScript } from '../browser-scripts/scripts.ts';
import type { BrowserPage } from '../node/browser-tasks.ts';

type Message = { id?: number; method?: string; params?: Record<string, unknown>; sessionId?: string; result?: Record<string, unknown>; error?: { message: string } };

export class CdpConnection {
  private nextId = 1;
  private pending = new Map<number, { resolve(value: Record<string, unknown>): void; reject(error: Error): void }>();
  private listeners = new Set<(message: Message) => void>();
  private socket: WebSocket;
  private constructor(socket: WebSocket) {
    this.socket = socket;
    socket.addEventListener('message', event => {
      const message = JSON.parse(String(event.data)) as Message;
      if (message.id !== undefined) { const waiter = this.pending.get(message.id); this.pending.delete(message.id); if (message.error) waiter?.reject(new Error(message.error.message)); else waiter?.resolve(message.result ?? {}); }
      else for (const listener of this.listeners) listener(message);
    });
    socket.addEventListener('close', () => { for (const waiter of this.pending.values()) waiter.reject(new Error('CDP connection closed')); this.pending.clear(); });
  }
  static async connect(url: string) {
    const socket = new WebSocket(url);
    await new Promise<void>((resolve, reject) => { socket.addEventListener('open', () => resolve(), { once: true }); socket.addEventListener('error', () => reject(new Error(`cannot connect to ${url}`)), { once: true }); });
    return new CdpConnection(socket);
  }
  send(method: string, params: Record<string, unknown> = {}, sessionId?: string) {
    const id = this.nextId++;
    return new Promise<Record<string, unknown>>((resolve, reject) => { this.pending.set(id, { resolve, reject }); this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) })); });
  }
  // Registered before the command that triggers the event, so the event cannot be missed.
  waitForEvent(match: (message: Message) => boolean, label: string, timeoutMs: number) {
    let listener: (message: Message) => void = () => {}; let timer: NodeJS.Timeout | undefined;
    const promise = new Promise<Message>((resolve, reject) => {
      listener = message => { if (match(message)) { this.listeners.delete(listener); clearTimeout(timer); resolve(message); } };
      timer = setTimeout(() => { this.listeners.delete(listener); reject(new Error(`${label} timeout`)); }, timeoutMs);
      this.listeners.add(listener);
    });
    return { promise, cancel: () => { this.listeners.delete(listener); clearTimeout(timer); promise.catch(() => {}); } };
  }
  waitFor(method: string, sessionId: string, timeoutMs: number) {
    let listener: (message: Message) => void = () => {}; let timer: NodeJS.Timeout | undefined;
    const promise = new Promise<void>((resolve, reject) => {
      listener = message => { if (message.method === method && message.sessionId === sessionId) { this.listeners.delete(listener); clearTimeout(timer); resolve(); } };
      timer = setTimeout(() => { this.listeners.delete(listener); reject(new Error(`${method} timeout`)); }, timeoutMs);
      this.listeners.add(listener);
    });
    return { promise, cancel: () => { this.listeners.delete(listener); clearTimeout(timer); } };
  }
  close() { this.socket.close(); }
}

export class CdpPage implements BrowserPage {
  commands = 0;
  private connection: CdpConnection; private targetId: string; private sessionId: string; private timeoutMs: number;
  constructor(connection: CdpConnection, targetId: string, sessionId: string, timeoutMs: number) { this.connection = connection; this.targetId = targetId; this.sessionId = sessionId; this.timeoutMs = timeoutMs; }
  static async open(connection: CdpConnection, timeoutMs: number) {
    const { targetId } = await connection.send('Target.createTarget', { url: 'about:blank' }) as { targetId: string };
    const { sessionId } = await connection.send('Target.attachToTarget', { targetId, flatten: true }) as { sessionId: string };
    const page = new CdpPage(connection, targetId, sessionId, timeoutMs); page.commands = 2;
    await page.send('Page.enable');
    await page.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false });
    return page;
  }
  // Every command for this page counts, including those on its browser connection and on frame sessions.
  send(method: string, params: Record<string, unknown> = {}, sessionId: string | null = this.sessionId) { this.commands++; return this.connection.send(method, params, sessionId ?? undefined); }
  async goto(url: string) {
    const loaded = this.connection.waitFor('Page.domContentEventFired', this.sessionId, this.timeoutMs);
    try { const result = await this.send('Page.navigate', { url }) as { errorText?: string }; if (result.errorText) throw new Error(`navigation failed: ${result.errorText}`); await loaded.promise; }
    catch (error) { loaded.cancel(); loaded.promise.catch(() => {}); throw error; }
  }
  async evaluate<T>(expression: string): Promise<T> {
    const result = await this.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true }) as { result?: { value?: T }; exceptionDetails?: { text?: string; exception?: { description?: string } } };
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text ?? 'evaluation failed');
    return result.result?.value as T;
  }
  async click(selector: string) {
    const { x, y } = await this.evaluate<{ x: number; y: number }>(pointScript(selector));
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
    await this.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
    await this.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
  }
  private async mouse(type: string, x: number, y: number, extra: Record<string, unknown> = {}) { await this.send('Input.dispatchMouseEvent', { type, x, y, ...extra }); }
  async hover(selector: string) { const { x, y } = await this.evaluate<{ x: number; y: number }>(pointScript(selector)); await this.mouse('mouseMoved', x, y); }
  async dblclick(selector: string) {
    const { x, y } = await this.evaluate<{ x: number; y: number }>(pointScript(selector)); await this.mouse('mouseMoved', x, y);
    for (const clickCount of [1, 2]) { await this.mouse('mousePressed', x, y, { button: 'left', buttons: 1, clickCount }); await this.mouse('mouseReleased', x, y, { button: 'left', buttons: 0, clickCount }); }
  }
  async rightClick(selector: string) {
    const { x, y } = await this.evaluate<{ x: number; y: number }>(pointScript(selector)); await this.mouse('mouseMoved', x, y);
    await this.mouse('mousePressed', x, y, { button: 'right', buttons: 2, clickCount: 1 }); await this.mouse('mouseReleased', x, y, { button: 'right', buttons: 0, clickCount: 1 });
  }
  async drag(from: string, to: string) {
    const start = await this.evaluate<{ x: number; y: number }>(pointScript(from)), end = await this.evaluate<{ x: number; y: number }>(pointScript(to));
    await this.mouse('mouseMoved', start.x, start.y); await this.mouse('mousePressed', start.x, start.y, { button: 'left', buttons: 1, clickCount: 1 });
    await this.mouse('mouseMoved', end.x, end.y, { buttons: 1 }); await this.mouse('mouseReleased', end.x, end.y, { button: 'left', buttons: 0, clickCount: 1 });
  }
  async wheel(selector: string, deltaY: number) { const { x, y } = await this.evaluate<{ x: number; y: number }>(pointScript(selector)); await this.mouse('mouseMoved', x, y); await this.mouse('mouseWheel', x, y, { deltaX: 0, deltaY }); }
  async press(selector: string, key: 'Enter') {
    if (key !== 'Enter') throw new Error(`unsupported key ${key}`);
    await this.evaluate(focusScript(selector));
    await this.send('Input.dispatchKeyEvent', { type: 'keyDown', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13, text: '\r' });
    await this.send('Input.dispatchKeyEvent', { type: 'keyUp', key: 'Enter', code: 'Enter', windowsVirtualKeyCode: 13 });
  }
  async goBack() {
    const history = await this.send('Page.getNavigationHistory') as { currentIndex: number; entries: { id: number }[] };
    const entry = history.entries[history.currentIndex - 1]; if (!entry) throw new Error('no previous history entry');
    // A back-forward cache restore fires frameNavigated without DOMContentLoaded, so wait for the main frame commit.
    const navigated = this.connection.waitForEvent(message => message.method === 'Page.frameNavigated' && message.sessionId === this.sessionId && !(message.params?.frame as { parentId?: string })?.parentId, 'Page.frameNavigated', this.timeoutMs);
    try { await this.send('Page.navigateToHistoryEntry', { entryId: entry.id }); await navigated.promise; } catch (error) { navigated.cancel(); throw error; }
  }
  async withPopup(trigger: string, act: (popup: BrowserPage) => Promise<void>) {
    await this.send('Target.setDiscoverTargets', { discover: true }, null);
    const created = this.connection.waitForEvent(message => message.method === 'Target.targetCreated' && (message.params?.targetInfo as { openerId?: string; type?: string })?.openerId === this.targetId && (message.params?.targetInfo as { type?: string })?.type === 'page', 'popup', this.timeoutMs);
    let targetId: string;
    try { await this.click(trigger); targetId = ((await created.promise).params!.targetInfo as { targetId: string }).targetId; } catch (error) { created.cancel(); throw error; }
    const { sessionId } = await this.send('Target.attachToTarget', { targetId, flatten: true }, null) as { sessionId: string };
    const popup = new CdpPage(this.connection, targetId, sessionId, this.timeoutMs);
    try { await act(popup); } finally { this.commands += popup.commands + 1; await this.connection.send('Target.closeTarget', { targetId }); }
  }
  async withFrame(selector: string, act: (frame: BrowserPage) => Promise<void>) {
    const offset = await this.evaluate<{ x: number; y: number }>(frameOffsetScript(selector));
    const { root } = await this.send('DOM.getDocument', { depth: 0 }) as { root: { nodeId: number } };
    const { nodeId } = await this.send('DOM.querySelector', { nodeId: root.nodeId, selector }) as { nodeId: number };
    const { node } = await this.send('DOM.describeNode', { nodeId }) as { node: { frameId?: string } };
    if (!node.frameId) throw new Error(`no frame for ${selector}`);
    // An in-process frame gets an isolated world; an out-of-process frame is its own target, reached by auto-attach.
    let frame: CdpFrame;
    try { const { executionContextId } = await this.send('Page.createIsolatedWorld', { frameId: node.frameId, worldName: 'automan' }) as { executionContextId: number }; frame = new CdpFrame(this, this.sessionId, executionContextId, offset); }
    catch {
      const attached = this.connection.waitForEvent(message => message.method === 'Target.attachedToTarget' && (message.params?.targetInfo as { targetId?: string })?.targetId === node.frameId, 'frame attach', this.timeoutMs);
      try { await this.send('Target.setAutoAttach', { autoAttach: true, waitForDebuggerOnStart: false, flatten: true }); frame = new CdpFrame(this, (await attached.promise).params!.sessionId as string, null, offset); } catch (error) { attached.cancel(); throw error; }
    }
    await act(frame);
  }
  async upload(selector: string, path: string) {
    const { root } = await this.send('DOM.getDocument', { depth: 0 }) as { root: { nodeId: number } };
    const { nodeId } = await this.send('DOM.querySelector', { nodeId: root.nodeId, selector }) as { nodeId: number };
    await this.send('DOM.setFileInputFiles', { files: [path], nodeId });
  }
  async download(selector: string) {
    const directory = await mkdtemp(join(tmpdir(), 'automan-download-'));
    await this.send('Browser.setDownloadBehavior', { behavior: 'allowAndName', downloadPath: directory, eventsEnabled: true }, null);
    const completed = this.connection.waitForEvent(message => message.method === 'Browser.downloadProgress' && message.params?.state === 'completed', 'download', this.timeoutMs);
    let guid: string;
    try { await this.click(selector); guid = (await completed.promise).params!.guid as string; } catch (error) { completed.cancel(); throw error; }
    return readFile(join(directory, guid), 'utf8');
  }
  async fill(selector: string, value: string) { await this.evaluate(focusScript(selector)); await this.send('Input.insertText', { text: value }); }
  async select(selector: string, value: string) { await this.evaluate(selectScript(selector, value)); }
  async close() { this.commands++; await this.connection.send('Target.closeTarget', { targetId: this.targetId }); }
}

// A frame of a CDP page: scripts run in its own context, input goes through the page in page coordinates.
class CdpFrame implements BrowserPage {
  private page: CdpPage; private sessionId: string; private contextId: number | null; private offset: { x: number; y: number };
  constructor(page: CdpPage, sessionId: string, contextId: number | null, offset: { x: number; y: number }) { this.page = page; this.sessionId = sessionId; this.contextId = contextId; this.offset = offset; }
  async evaluate<T>(expression: string): Promise<T> {
    const result = await this.page.send('Runtime.evaluate', { expression, awaitPromise: true, returnByValue: true, ...(this.contextId !== null ? { contextId: this.contextId } : {}) }, this.sessionId) as { result?: { value?: T }; exceptionDetails?: { text?: string; exception?: { description?: string } } };
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.exception?.description ?? result.exceptionDetails.text ?? 'evaluation failed');
    return result.result?.value as T;
  }
  async click(selector: string) {
    const point = await this.evaluate<{ x: number; y: number }>(pointScript(selector)); const [x, y] = [point.x + this.offset.x, point.y + this.offset.y];
    await this.page.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x, y });
    await this.page.send('Input.dispatchMouseEvent', { type: 'mousePressed', x, y, button: 'left', buttons: 1, clickCount: 1 });
    await this.page.send('Input.dispatchMouseEvent', { type: 'mouseReleased', x, y, button: 'left', buttons: 0, clickCount: 1 });
  }
  goto(): Promise<void> { throw new Error('navigation inside a frame is not part of the frame contract'); }
  fill(): Promise<void> { throw new Error('fill inside a frame is not part of the frame contract'); }
  select(): Promise<void> { throw new Error('select inside a frame is not part of the frame contract'); }
  async close() {}
}
