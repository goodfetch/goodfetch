// In-process SDK: the engine is the napi-rs addon loaded into this Node process.
import { createRequire } from 'node:module';
import type { Engine, RunHooks, Summary } from './types.ts';

const require = createRequire(import.meta.url);
const addon = require('../napi-addon/automan_napi.node');
let seq = 0;

export class NativeEngine implements Engine {
  async run(opts: Record<string, unknown>, hooks: RunHooks): Promise<Summary> {
    const runId = (opts.run_id as string) ?? `run-${process.pid}-${seq++}`;
    const onAbort = () => addon.cancel(runId);
    hooks.signal?.addEventListener('abort', onAbort);
    try {
      const out = await addon.run(JSON.stringify({ ...opts, run_id: runId }), hooks.handler, hooks.onEvent ?? (() => {}));
      return JSON.parse(out);
    } finally {
      hooks.signal?.removeEventListener('abort', onAbort);
    }
  }
  async close() {}
}
