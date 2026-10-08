// Minimal Chrome DevTools Protocol client over the runtime's WebSocket. The command sequence per page
// operation is the contract shared with the Rust driver (adapters/cdp-rust/src/cdp.rs).
import { pointScript, focusScript, selectScript } from '../browser-scripts/scripts.ts';
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
  private constructor(connection: CdpConnection, targetId: string, sessionId: string, timeoutMs: number) { this.connection = connection; this.targetId = targetId; this.sessionId = sessionId; this.timeoutMs = timeoutMs; }
  static async open(connection: CdpConnection, timeoutMs: number) {
    const { targetId } = await connection.send('Target.createTarget', { url: 'about:blank' }) as { targetId: string };
    const { sessionId } = await connection.send('Target.attachToTarget', { targetId, flatten: true }) as { sessionId: string };
    const page = new CdpPage(connection, targetId, sessionId, timeoutMs); page.commands = 2;
    await page.send('Page.enable');
    await page.send('Emulation.setDeviceMetricsOverride', { width: 1280, height: 720, deviceScaleFactor: 1, mobile: false });
    return page;
  }
  private send(method: string, params: Record<string, unknown> = {}) { this.commands++; return this.connection.send(method, params, this.sessionId); }
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
  async fill(selector: string, value: string) { await this.evaluate(focusScript(selector)); await this.send('Input.insertText', { text: value }); }
  async select(selector: string, value: string) { await this.evaluate(selectScript(selector, value)); }
  async close() { this.commands++; await this.connection.send('Target.closeTarget', { targetId: this.targetId }); }
}
