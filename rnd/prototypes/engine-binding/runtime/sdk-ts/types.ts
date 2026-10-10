// Shared SDK surface: both bindings expose `run(opts, hooks)`.
import { readFileSync } from 'node:fs';

export type BlobRef = { path: string; size: number; sha256: string };
export type Page = { run_id: string; idx: number; url: string; body?: Buffer | string; blob: BlobRef; t0_us: number };
export type EngineEvent = { kind: string; run_id: string; seq: number; idx: number; t_us: number; data: string };
export type Summary = Record<string, any> & { pages_done: number; cancelled: boolean; errors: string[] };
export type Handler = (page: Page, signal?: AbortSignal) => Promise<unknown>;
export type RunHooks = { handler: Handler; onEvent?: (evs: EngineEvent[]) => void; signal?: AbortSignal };
export interface Engine {
  run(opts: Record<string, unknown>, hooks: RunHooks): Promise<Summary>;
  close(): Promise<void>;
}

/** Page body: inline if delivered, otherwise read from the content-addressed blob. */
export function readBody(p: Page): Buffer | string {
  return p.body ?? readFileSync(p.blob.path);
}

export const nowUs = () => (performance.timeOrigin + performance.now()) * 1000;
