// RPC SDK: newline-delimited JSON-RPC 2.0 to `engined`, as a stdio sidecar or over a daemon's Unix socket.
import { spawn, type ChildProcess } from 'node:child_process';
import { createConnection } from 'node:net';
import type { Duplex, Writable, Readable } from 'node:stream';
import type { Engine, RunHooks, Summary } from './types.ts';

export const PROTOCOL_VERSION = '1.0';
let seq = 0;

export class RpcError extends Error {
  code: number;
  data?: unknown;
  constructor(code: number, message: string, data?: unknown) { super(message); this.code = code; this.data = data; }
}

export class RpcEngine implements Engine {
  private nextId = 1;
  private pending = new Map<number, { resolve: (v: any) => void; reject: (e: Error) => void }>();
  private runs = new Map<string, RunHooks>();
  private inflight = new Map<number, AbortController>();
  info: any;
  private out: Writable;
  child?: ChildProcess;

  private constructor(out: Writable, input: Readable, child?: ChildProcess) {
    this.out = out;
    this.child = child;
    let parts: Buffer[] = [];
    input.on('data', (chunk: Buffer) => {
      let start = 0, i: number;
      while ((i = chunk.indexOf(10, start)) >= 0) {
        parts.push(chunk.subarray(start, i));
        const line = parts.length === 1 ? parts[0] : Buffer.concat(parts);
        parts = [];
        start = i + 1;
        this.dispatch(JSON.parse(line.toString('utf8')));
      }
      if (start < chunk.length) parts.push(chunk.subarray(start));
    });
    input.on('close', () => {
      for (const p of this.pending.values()) p.reject(new Error('engine connection closed'));
      this.pending.clear();
    });
  }

  /** Spawns a private engine process (sidecar) speaking over stdin/stdout. */
  static async sidecar(bin: string, protocolVersion = PROTOCOL_VERSION) {
    const child = spawn(bin, ['stdio'], { stdio: ['pipe', 'pipe', 'inherit'] });
    const e = new RpcEngine(child.stdin!, child.stdout!, child);
    e.info = await e.request('initialize', { protocolVersion, sdk: 'node' });
    return e;
  }

  /** Connects to a shared engine daemon. */
  static async connect(sock: string, protocolVersion = PROTOCOL_VERSION) {
    const s: Duplex = createConnection(sock);
    await new Promise((res, rej) => { s.once('connect', res); s.once('error', rej); });
    const e = new RpcEngine(s, s);
    e.info = await e.request('initialize', { protocolVersion, sdk: 'node' });
    return e;
  }

  private send(msg: object) {
    this.out.write(JSON.stringify({ jsonrpc: '2.0', ...msg }) + '\n');
  }

  request(method: string, params: unknown, id = this.nextId++): Promise<any> {
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.send({ id, method, params });
    });
  }

  private dispatch(m: any) {
    switch (m.method) {
      case 'handle': {
        const hooks = this.runs.get(m.params.run_id);
        const ac = new AbortController();
        this.inflight.set(m.id, ac);
        Promise.resolve()
          .then(() => hooks!.handler(m.params, ac.signal))
          .then(
            (result) => this.send({ id: m.id, result: result ?? null }),
            (err: any) => this.send({ id: m.id, error: { code: -32000, message: String(err?.message ?? err), data: { stack: err?.stack } } }),
          )
          .finally(() => this.inflight.delete(m.id));
        return;
      }
      case 'events':
        if (m.params.length) this.runs.get(m.params[0].run_id)?.onEvent?.(m.params);
        return;
      case '$/cancelRequest':
        this.inflight.get(m.params.id)?.abort();
        return;
      case undefined: {
        const p = this.pending.get(m.id);
        this.pending.delete(m.id);
        if (m.error) p?.reject(new RpcError(m.error.code, m.error.message, m.error.data));
        else p?.resolve(m.result);
      }
    }
  }

  async run(opts: Record<string, unknown>, hooks: RunHooks): Promise<Summary> {
    const runId = (opts.run_id as string) ?? `run-${process.pid}-${seq++}`;
    const id = this.nextId++;
    const onAbort = () => this.send({ method: '$/cancelRequest', params: { id } });
    this.runs.set(runId, hooks);
    hooks.signal?.addEventListener('abort', onAbort);
    try {
      return await this.request('run', { ...opts, run_id: runId }, id);
    } finally {
      hooks.signal?.removeEventListener('abort', onAbort);
      this.runs.delete(runId);
    }
  }

  async close() {
    this.out.end();
    if (this.child && this.child.exitCode === null) await new Promise((r) => this.child!.once('exit', r));
  }
}
