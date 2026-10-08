import { spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { once } from 'node:events';
import { LabError } from './core/errors.ts';

export type WorkerCommand = { command: string; args: string[]; cwd?: string; timeoutMs?: number; env?: NodeJS.ProcessEnv };
export class WorkerClient {
  private child: ChildProcessWithoutNullStreams;
  private pending = new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void; timer: NodeJS.Timeout }>();
  private sequence = 0;
  private closed = false;
  private timeout: number;
  private buffer = '';
  stderr = '';
  readonly pid: number;
  constructor(spec: WorkerCommand) {
    this.timeout = spec.timeoutMs ?? 30_000;
    this.child = spawn(spec.command, spec.args, { cwd: spec.cwd, env: spec.env, stdio: ['pipe', 'pipe', 'pipe'], detached: process.platform !== 'win32' });
    this.pid = this.child.pid ?? 0;
    this.child.stdout.setEncoding('utf8'); this.child.stderr.setEncoding('utf8');
    this.child.stdout.on('data', (data: string) => {
      this.buffer += data;
      if (this.buffer.length > 8_000_000) return this.fail(new LabError('E_OUTPUT_LIMIT', 'adapter output exceeds limit'));
      let index: number;
      while ((index = this.buffer.indexOf('\n')) >= 0) {
        const line = this.buffer.slice(0, index); this.buffer = this.buffer.slice(index + 1);
        try {
          const message = JSON.parse(line);
          if (!message || !Number.isInteger(message.id) || message.version !== 1 || (message.error === undefined && message.result === undefined)) throw new Error('invalid response');
          const wait = this.pending.get(message.id); if (!wait) continue;
          clearTimeout(wait.timer); this.pending.delete(message.id);
          if (message.error !== undefined) wait.reject(new LabError('E_ADAPTER_ERROR', String(message.error))); else wait.resolve(message.result);
        } catch { this.fail(new LabError('E_PROTOCOL', 'invalid JSONL adapter response')); return; }
      }
    });
    this.child.stderr.on('data', (data: string) => { this.stderr = (this.stderr + data).slice(-16_000); });
    this.child.on('error', error => this.fail(new LabError('E_ADAPTER_EXIT', error.message, { cause: error })));
    this.child.on('exit', (code, signal) => this.fail(new LabError('E_ADAPTER_EXIT', `adapter exited (${code ?? signal})`)));
    this.child.stdin.on('error', error => this.fail(new LabError('E_ADAPTER_EXIT', error.message, { cause: error })));
  }
  async call(method: string, params: unknown, timeoutMs = this.timeout): Promise<unknown> {
    if (this.closed) throw new LabError('E_ADAPTER_CLOSED', 'adapter closed');
    if (this.pending.size >= 128) throw new LabError('E_CAPACITY', 'adapter outstanding request limit');
    const id = ++this.sequence; const line = JSON.stringify({ version: 1, id, method, params }) + '\n';
    if (line.length > 128_000) throw new LabError('E_OUTPUT_LIMIT', 'adapter request exceeds limit');
    return await new Promise((resolve, reject) => {
      const timer = setTimeout(() => this.fail(new LabError('E_ADAPTER_TIMEOUT', `adapter timeout (${method})`), id), timeoutMs);
      this.pending.set(id, { resolve, reject, timer });
      this.child.stdin.write(line, error => { if (error) this.fail(new LabError('E_ADAPTER_EXIT', error.message, { cause: error })); });
    });
  }
  private signal(signal: NodeJS.Signals) {
    if (!this.pid) return;
    try { if (process.platform === 'win32') this.child.kill(signal); else process.kill(-this.pid, signal); } catch { /* already stopped */ }
  }
  get isClosed() { return this.closed; }
  // On a timeout only the call that exceeded its deadline carries the timeout; calls in flight beside it are aborted siblings.
  private fail(error: Error, rootId?: number) {
    if (this.closed) return;
    this.closed = true;
    for (const [id, wait] of this.pending) { clearTimeout(wait.timer); wait.reject(rootId === undefined || id === rootId ? error : new LabError('E_SIBLING_ABORT', `aborted by ${error.message}`, { cause: error })); }
    this.pending.clear(); this.signal('SIGTERM');
    const escalation = setTimeout(() => this.signal('SIGKILL'), 300); escalation.unref();
  }
  async close() {
    if (!this.closed) { try { await this.call('close', {}, 1000); } catch { /* force-stop below */ } }
    this.fail(new LabError('E_ADAPTER_CLOSED', 'adapter closed'));
    this.signal('SIGTERM');
    if (this.child.exitCode === null && this.child.signalCode === null) {
      let deadline:NodeJS.Timeout|undefined;
      try{await Promise.race([once(this.child, 'exit').catch(() => {}), new Promise(resolve => {deadline=setTimeout(resolve,400)})])}
      finally{if(deadline)clearTimeout(deadline)}
    }
    this.signal('SIGKILL');
    this.child.stdin.destroy(); this.child.stdout.destroy(); this.child.stderr.destroy();
  }
}
