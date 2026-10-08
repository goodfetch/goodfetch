import { createInterface } from 'node:readline';
export type Task = { scenario: string; url: string; seed: number; pages: number; actions: number; timeoutMs: number; variant: string };
export type AdapterOutput = { status: 'ok' | 'unsupported'; records?: { id: string; price: number }[]; file?: string; metrics?: Record<string, number>; reason?: string };
export interface Adapter<RunTask = Task, PrepareOptions = { executablePath?: string; headless?: boolean }> {
  info(): Record<string, unknown>;
  prepare(options: PrepareOptions): Promise<void>;
  run(task: RunTask): Promise<AdapterOutput | Record<string, unknown>>;
  close(): Promise<void>;
}
export function serve<RunTask, PrepareOptions>(adapter: Adapter<RunTask, PrepareOptions>) {
  const lines = createInterface({ input: process.stdin }); let closing = false;
  async function handle(line: string) {
    let id: number | null = null;
    try {
      if (line.length > 128_000) throw new Error('request exceeds limit');
      const message = JSON.parse(line); id = message.id;
      if (message.version !== 1 || !Number.isInteger(id) || closing) throw new Error('invalid request');
      let result: unknown;
      if (message.method === 'hello') result = adapter.info();
      else if (message.method === 'prepare') { await adapter.prepare(message.params); result = { ready: true }; }
      else if (message.method === 'run') result = await adapter.run(message.params);
      else if (message.method === 'close') { closing = true; await adapter.close(); result = { closed: true }; }
      else throw new Error('unknown method');
      process.stdout.write(JSON.stringify({ version: 1, id, result }) + '\n');
      if (closing) { lines.close(); process.stdin.destroy(); }
    } catch (error) { process.stdout.write(JSON.stringify({ version: 1, id, error: error instanceof Error ? error.message : String(error) }) + '\n'); }
  }
  lines.on('line', line => { void handle(line); });
  lines.on('close', () => { if (!closing) void adapter.close().finally(() => process.exit(0)); });
  process.on('SIGTERM', () => { void adapter.close().finally(() => process.exit(0)); });
}
