// A WebDriver BiDi client for the Firefox installed on the machine, over Node's own WebSocket: it starts Firefox with
// its remote agent on a fresh profile, opens a BiDi-only session and keeps the log of every command it sent, which is
// what the report reads the commands per click from (issue 2460).
import { spawn, type ChildProcess } from 'node:child_process';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

// One command as sent: its method and, for a script call, the realm it targeted.
export type Sent = { method: string; sandbox?: string; context?: string };
type Pending = { resolve: (value: Record<string, unknown>) => void; reject: (error: Error) => void };
type Listener = (params: Record<string, unknown>) => void;

// An error the browser answered, with its WebDriver error code (`no such frame`, `unknown error`...).
export class BidiError extends Error {
  readonly method: string;
  readonly code: string;
  constructor(method: string, code: string, message: string) { super(`${method}: ${code}: ${message}`); this.method = method; this.code = code; }
}

export class Bidi {
  readonly sent: Sent[] = [];
  private readonly pending = new Map<number, Pending>();
  private readonly listeners = new Set<{ event: string; listener: Listener }>();
  private sequence = 0;
  private readonly socket: WebSocket;
  private readonly browser: ChildProcess;
  private readonly profile: string;
  version = 'unknown';
  private constructor(socket: WebSocket, browser: ChildProcess, profile: string) {
    this.socket = socket; this.browser = browser; this.profile = profile;
    socket.addEventListener('message', message => this.receive(String(message.data)));
    socket.addEventListener('close', () => this.failAll(new Error('the BiDi connection closed')));
  }

  // Starts `executablePath` headless on a fresh profile and opens a session on its BiDi endpoint.
  static async launch(executablePath: string, prefs: Record<string, string | number | boolean> = {}): Promise<Bidi> {
    const profile = await mkdtemp(join(tmpdir(), 'goodfetch-firefox-bidi-'));
    const userPrefs = { 'browser.shell.checkDefaultBrowser': false, 'browser.startup.homepage_override.mstone': 'ignore', 'datareporting.policy.dataSubmissionEnabled': false, ...prefs };
    await writeFile(join(profile, 'user.js'), Object.entries(userPrefs).map(([key, value]) => `user_pref(${JSON.stringify(key)}, ${JSON.stringify(value)});`).join('\n') + '\n');
    const browser = spawn(executablePath, ['--headless', '--no-remote', '--profile', profile, '--remote-debugging-port=0', 'about:blank'], { stdio: ['ignore', 'ignore', 'pipe'] });
    try {
      const endpoint = await new Promise<string>((resolve, reject) => {
        let text = '';
        const timer = setTimeout(() => reject(new Error(`Firefox printed no BiDi endpoint in 30 s: ${text.slice(-2000)}`)), 30_000);
        browser.once('exit', code => { clearTimeout(timer); reject(new Error(`Firefox exited (${code}) before its BiDi endpoint: ${text.slice(-2000)}`)); });
        browser.stderr!.setEncoding('utf8');
        browser.stderr!.on('data', (data: string) => {
          text = (text + data).slice(-16_000);
          const found = text.match(/WebDriver BiDi listening on (ws:\/\/\S+)/);
          if (found) { clearTimeout(timer); resolve(found[1]!); }
        });
      });
      const socket = new WebSocket(`${endpoint}/session`);
      await new Promise<void>((resolve, reject) => { socket.addEventListener('open', () => resolve(), { once: true }); socket.addEventListener('error', () => reject(new Error(`no BiDi connection to ${endpoint}`)), { once: true }); });
      const bidi = new Bidi(socket, browser, profile);
      const session = await bidi.send('session.new', { capabilities: { alwaysMatch: { acceptInsecureCerts: true } } });
      const capabilities = session.capabilities as { browserVersion?: string };
      bidi.version = capabilities.browserVersion ?? 'unknown';
      return bidi;
    } catch (error) { browser.kill('SIGKILL'); await rm(profile, { recursive: true, force: true }); throw error; }
  }

  // Sends one command and answers its result; every command is logged as sent, before its answer. A command left
  // without an answer (a stalled browser, a dropped context) fails after `timeoutMs` instead of waiting forever.
  send(method: string, params: Record<string, unknown> = {}, timeoutMs = 60_000): Promise<Record<string, unknown>> {
    const id = ++this.sequence;
    const target = params.target as { context?: string; sandbox?: string } | undefined;
    this.sent.push({ method, ...(target?.sandbox !== undefined ? { sandbox: target.sandbox } : {}), ...(target?.context !== undefined ? { context: target.context } : {}) });
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`${method}: no answer within ${timeoutMs} ms`)); }, timeoutMs);
      this.pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); } });
      this.socket.send(JSON.stringify({ id, method, params }));
    });
  }

  // Calls `listener` on each `event` until the returned function removes it.
  on(event: string, listener: Listener): () => void {
    const entry = { event, listener }; this.listeners.add(entry);
    return () => { this.listeners.delete(entry); };
  }

  // Waits for the first `event` that `accept` takes, failing after `timeoutMs`. Registered before the command that
  // causes the event, so the event cannot be missed; `cancel` stops the wait when that command failed, and the wait's
  // own failure is then dropped.
  next(event: string, accept: (params: Record<string, unknown>) => boolean, timeoutMs: number): { promise: Promise<Record<string, unknown>>; cancel(): void } {
    let off = () => {}; let timer: NodeJS.Timeout | undefined;
    const promise = new Promise<Record<string, unknown>>((resolve, reject) => {
      timer = setTimeout(() => { off(); reject(new Error(`no ${event} within ${timeoutMs} ms`)); }, timeoutMs);
      off = this.on(event, params => { if (accept(params)) { clearTimeout(timer); off(); resolve(params); } });
    });
    return { promise, cancel: () => { off(); clearTimeout(timer); promise.catch(() => {}); } };
  }

  async close(): Promise<void> {
    if (this.running()) await this.send('browser.close', {}, 3000).catch(() => { /* gone, or stalled: killed below */ });
    this.socket.close();
    if (this.running()) {
      const exited = new Promise(resolve => this.browser.once('exit', resolve));
      const timer = setTimeout(() => this.browser.kill('SIGKILL'), 5000);
      await exited; clearTimeout(timer);
    }
    await rm(this.profile, { recursive: true, force: true });
  }

  private receive(text: string) {
    const message = JSON.parse(text) as { type: string; id?: number; result?: Record<string, unknown>; error?: string; message?: string; method?: string; params?: Record<string, unknown> };
    if (message.type === 'event') { for (const { event, listener } of [...this.listeners]) if (event === message.method) listener(message.params ?? {}); return; }
    if (message.id === undefined) return;
    const wait = this.pending.get(message.id); if (!wait) return;
    this.pending.delete(message.id);
    if (message.type === 'success') wait.resolve(message.result ?? {});
    else wait.reject(new BidiError(this.sent[message.id - 1]?.method ?? 'unknown', message.error ?? 'unknown error', message.message ?? ''));
  }

  // A browser that exited with a code or was killed by a signal has emitted its `exit` already.
  private running(): boolean { return this.browser.exitCode === null && this.browser.signalCode === null; }

  private failAll(error: Error) { for (const wait of this.pending.values()) wait.reject(error); this.pending.clear(); }
}

// A local value as BiDi serializes an argument.
export function toLocal(value: unknown): Record<string, unknown> {
  if (value === undefined) return { type: 'undefined' };
  if (value === null) return { type: 'null' };
  if (typeof value === 'string') return { type: 'string', value };
  if (typeof value === 'number') return { type: 'number', value };
  if (typeof value === 'boolean') return { type: 'boolean', value };
  if (Array.isArray(value)) return { type: 'array', value: value.map(toLocal) };
  if (typeof value === 'object') return { type: 'object', value: Object.entries(value).filter(([, item]) => item !== undefined).map(([key, item]) => [key, toLocal(item)]) };
  throw new Error(`no BiDi local value for ${typeof value}`);
}

// A remote value as plain JSON; a node or a window keeps its reference fields.
export function fromRemote(remote: Record<string, unknown>): unknown {
  const value = remote.value;
  switch (remote.type) {
    case 'undefined': case 'null': return null;
    case 'string': case 'boolean': return value;
    case 'number': return typeof value === 'number' ? value : Number(value);
    case 'array': return (value as Record<string, unknown>[]).map(fromRemote);
    case 'object': return Object.fromEntries((value as [string | Record<string, unknown>, Record<string, unknown>][]).map(([key, item]) => [typeof key === 'string' ? key : String(fromRemote(key)), fromRemote(item)]));
    default: return remote;
  }
}
