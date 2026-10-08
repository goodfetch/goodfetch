// Unified campaign engine (bench lane): provenance → host gate → conformance → warmup → measured blocks → finalization.
import { appendFile } from 'node:fs/promises';
import { join } from 'node:path';
import { WorkerClient, type WorkerCommand } from '../worker-client.ts';
import { commandFor, type BackendName } from '../registry.ts';
import { shuffle, quantiles } from '../statistics.ts';
import { schedule } from '../scheduler.ts';
import { browserPath } from '../environment.ts';
import { sampleProcessGroup } from '../resources.ts';
import { sampleRuntimeResources } from '../runtime-resources.ts';
import { LabError, errorCode, statusFor, type AttemptStatus, type ErrorCode } from './errors.ts';
import { validateBenchCampaign, toLegacyBench, type BenchCampaign } from './campaign.ts';
import { startOracle, type Oracle } from './oracle.ts';
import { detectProbes, type Measure } from './metrics/registry.ts';
import { openRun, type RunOptions } from './run-context.ts';
import type { Scenario } from '../../fixtures/server.ts';

export type AttemptV2 = {
  attemptId: string; cellId: string; block: number; phase: 'conformance' | 'warmup' | 'measure'; index: number; seed: number;
  status: AttemptStatus; errorCode: ErrorCode | null; timing: { queueMs: number | null; taskMs: number | null; oracleMs: number | null };
  metrics: Record<string, Measure>; workerGeneration: number; reasons: string[]; adapterMetrics?: Record<string, number>; oracleSnapshot?: Record<string, unknown>;
};
type Counts = { verified: number; failed: number; timeout: number; crashed: number; aborted: number; unsupported: number; rejected: number; unavailable: number; skipped: number };
export type EngineOptions = RunOptions & { command?: (backend: string) => WorkerCommand; oracle?: Oracle | 'process' | 'in-process'; executablePath?: string };

const browserBackends = (backends: string[]) => backends.some(backend => !['node-http', 'crawlee-http', 'crawlee-http-pooled', 'spider-http'].includes(backend));

function checkedOutput(value: unknown) {
  const out = value as { status?: string; records?: { id: unknown; price: unknown }[]; file?: unknown; metrics?: Record<string, unknown>; reason?: string };
  if (!out || !['ok', 'unsupported'].includes(out.status ?? '')) throw new LabError('E_INVALID_OUTPUT', 'invalid adapter output');
  if (out.records && (!Array.isArray(out.records) || out.records.some(r => typeof r.id !== 'string' || !Number.isFinite(r.price)))) throw new LabError('E_INVALID_OUTPUT', 'invalid records');
  if (out.metrics !== undefined && (typeof out.metrics !== 'object' || out.metrics === null || Object.values(out.metrics).some(v => typeof v !== 'number' || !Number.isFinite(v) || v < 0))) throw new LabError('E_INVALID_OUTPUT', 'invalid metrics');
  return out as { status: 'ok' | 'unsupported'; metrics?: Record<string, number>; reason?: string };
}

// One adapter process per backend and block; restarted with a new generation after a crash or timeout, within maxRestarts.
class WorkerSlot {
  generation = -1; restarts = 0; capabilities: string[] = []; client: WorkerClient | undefined; samplers: { stop(): unknown }[] = []; startupMs: number[] = [];
  private starting: Promise<void> | undefined;
  private spec: WorkerCommand; private prepare: Record<string, unknown>; private maxRestarts: number; private concurrency: number; private sample: (pid: number) => { stop(): unknown };
  constructor(spec: WorkerCommand, prepare: Record<string, unknown>, maxRestarts: number, concurrency: number, sample: (pid: number) => { stop(): unknown }) {
    this.spec = spec; this.prepare = prepare; this.maxRestarts = maxRestarts; this.concurrency = concurrency; this.sample = sample;
  }
  async ensure() {
    if (this.client && !this.client.isClosed) return;
    if (!this.starting) this.starting = this.start().finally(() => { this.starting = undefined; });
    await this.starting;
  }
  private async start() {
    if (this.generation >= 0) { if (this.restarts >= this.maxRestarts) throw new LabError('E_RESTART_LIMIT', `worker restart limit ${this.maxRestarts} reached`); this.restarts++; }
    this.generation++;
    const began = performance.now(); const client = new WorkerClient(this.spec); this.client = client; this.samplers.push(this.sample(client.pid));
    try {
      const info = await client.call('hello', {}) as { capabilities?: unknown; maxConcurrency?: number };
      if (!Array.isArray(info.capabilities) || !info.capabilities.every(c => typeof c === 'string')) throw new LabError('E_PROTOCOL', 'invalid capabilities');
      if (this.concurrency > (info.maxConcurrency ?? 128)) throw new LabError('E_CAPABILITY', 'campaign exceeds adapter concurrency contract');
      this.capabilities = info.capabilities as string[];
      await client.call('prepare', this.prepare, Math.max(this.spec.timeoutMs ?? 30_000, 45_000));
    } catch (error) { await client.close(); throw new LabError('E_STARTUP', error instanceof Error ? error.message : String(error), { cause: error }); }
    finally { this.startupMs.push(performance.now() - began); }
  }
  // Samplers stop before the adapter exits so the final sample still sees the process tree.
  async close() { const resources = await Promise.all(this.samplers.map(sampler => sampler.stop())); await this.client?.close(); return resources; }
}

export async function runEngine(campaign: BenchCampaign, options: EngineOptions = {}) {
  validateBenchCampaign(campaign);
  const context = await openRun(campaign, options);
  const { directory, measureOf } = context;
  const probes = detectProbes();
  const oracle = typeof options.oracle === 'object' ? options.oracle : await startOracle(options.oracle ?? 'process');
  const ownsOracle = typeof options.oracle !== 'object';
  const legacy = toLegacyBench(campaign);
  const executablePath = options.executablePath ?? (browserBackends(campaign.backends) ? browserPath() : undefined);
  const attempts: AttemptV2[] = []; const windows: { cellId: string; durationMs: number }[] = []; const cellResources = new Map<string, unknown[]>();
  let writes = Promise.resolve();
  const record = (attempt: AttemptV2) => { attempts.push(attempt); writes = writes.then(() => appendFile(join(directory, 'attempts.v2.jsonl'), JSON.stringify(attempt) + '\n')); return writes; };
  const sample = (pid: number) => probes['proc_pid_rusage-v4'] ? sampleRuntimeResources(pid, 100) : sampleProcessGroup(pid);
  try {
    for (let block = 0; block < campaign.blocks; block++) {
      const order = shuffle(campaign.backends, campaign.seed + block);
      for (const backend of order) {
        const spec = options.command?.(backend) ?? commandFor(backend as BackendName, campaign.timeoutMs, executablePath);
        const slot = new WorkerSlot(spec, { executablePath, headless: true, concurrency: campaign.concurrency, poolWake: campaign.poolWake }, campaign.maxRestarts, campaign.concurrency, sample);
        try {
          for (const scenario of campaign.scenarios) {
            const cellId = `${backend}:${scenario}`;
            const make = (phase: AttemptV2['phase'], index: number, status: AttemptStatus, errorCodeValue: ErrorCode | null, reasons: string[], queueMs: number | null = null): AttemptV2 => ({
              attemptId: `${cellId}:b${block}:${phase}:${index}`, cellId, block, phase, index,
              seed: campaign.seed + block * 1_000_003 + index + (phase === 'measure' ? 10000 : phase === 'warmup' ? 1000 : 0),
              status, errorCode: errorCodeValue, timing: { queueMs, taskMs: null, oracleMs: null }, metrics: {}, workerGeneration: Math.max(0, slot.generation), reasons });
            const attempt = async (phase: AttemptV2['phase'], index: number, queueMs: number | null = null): Promise<AttemptStatus> => {
              const result = make(phase, index, 'failed', null, [], queueMs);
              try { await slot.ensure(); } catch (error) { const code = errorCode(error); Object.assign(result, { status: statusFor(code), errorCode: code, reasons: [String((error as Error).message)] }); await record(result); return result.status; }
              result.workerGeneration = slot.generation;
              if (!slot.capabilities.includes(scenario)) { Object.assign(result, { status: 'unsupported', errorCode: 'E_CAPABILITY', reasons: ['capability not advertised'] }); await record(result); return result.status; }
              const variant = campaign.fixtures.variants?.[index % campaign.fixtures.variants.length] ?? 'normal';
              const fixture = await oracle.createRun({ scenario: scenario as Scenario, seed: result.seed, pages: campaign.fixtures.pages, actions: campaign.fixtures.actions, delayMs: campaign.fixtures.delayMs, variant: variant as never });
              const client = slot.client!; const began = performance.now();
              try {
                const out = checkedOutput(await client.call('run', { ...legacy, scenario, seed: result.seed, variant, url: fixture.url }));
                result.timing.taskMs = performance.now() - began;
                if (out.metrics) result.adapterMetrics = out.metrics;
                if (out.status === 'unsupported') Object.assign(result, { status: 'unsupported', errorCode: 'E_CAPABILITY', reasons: [out.reason ?? 'unsupported'] });
                else {
                  const verifyStart = performance.now(); const verdict = await oracle.verify(fixture.id, out); result.timing.oracleMs = performance.now() - verifyStart;
                  Object.assign(result, { status: verdict.status, errorCode: verdict.status === 'failed' ? 'E_ORACLE_MISMATCH' : null, reasons: verdict.reasons });
                }
              } catch (error) {
                result.timing.taskMs ??= performance.now() - began;
                const code = errorCode(error); Object.assign(result, { status: statusFor(code), errorCode: code, reasons: [String((error as Error).message)] });
              }
              if (result.timing.taskMs !== null) result.metrics['task-ms'] = measureOf('task-ms', result.timing.taskMs, 'engine; run RPC only, oracle excluded');
              result.oracleSnapshot = await oracle.snapshot(fixture.id);
              await record(result); return result.status;
            };
            const skipRest = async (phase: AttemptV2['phase'], from: number, count: number) => { for (let i = 0; i < count; i++) await record(make(phase, from + i, 'skipped-gate', null, ['blocked by a failed gate'])); };
            const variants = campaign.fixtures.variants?.length ?? 1;
            let qualified = true;
            for (let i = 0; i < variants && qualified; i++) if (await attempt('conformance', i) !== 'verified') { qualified = false; await skipRest('conformance', i + 1, variants - i - 1); }
            for (let i = 0; i < campaign.warmup; i++) {
              if (!qualified) { await skipRest('warmup', i + 1, campaign.warmup - i); break; }
              if (await attempt('warmup', i + 1) !== 'verified') { qualified = false; await skipRest('warmup', i + 2, campaign.warmup - i - 1); }
            }
            if (!qualified) { await skipRest('measure', campaign.warmup + 1, campaign.iterations); continue; }
            const window = await schedule(campaign, (index, queueMs) => attempt('measure', index + campaign.warmup + 1, queueMs).then(() => undefined),
              async (index, queueMs) => { await record(make('measure', index + campaign.warmup + 1, 'rejected', 'E_CAPACITY', ['arrival exceeds admission or campaign deadline'], queueMs)); });
            windows.push({ cellId, durationMs: window.durationMs });
          }
        } finally {
          const resources = await slot.close();
          for (const scenario of campaign.scenarios) { const key = `${backend}:${scenario}`; cellResources.set(key, [...(cellResources.get(key) ?? []), ...(resources as unknown[])]); }
        }
      }
      await context.sampleBlock(block, order);
    }
  } finally { if (ownsOracle) await oracle.close(); }
  await writes;
  const cells = campaign.backends.flatMap(backend => campaign.scenarios.map(scenario => summarizeCell(`${backend}:${scenario}`, { backend, scenario }, attempts, windows, cellResources.get(`${backend}:${scenario}`) ?? [], measureOf)));
  const run = await context.close({ lane: 'bench', campaign, decisionIds: campaign.decisionIds, countUnit: 'attempt', cells, binaryChanges: null });
  return { directory, run, attempts };
}

function zero(): Counts { return { verified: 0, failed: 0, timeout: 0, crashed: 0, aborted: 0, unsupported: 0, rejected: 0, unavailable: 0, skipped: 0 }; }

function summarizeCell(cellId: string, factors: Record<string, string>, attempts: AttemptV2[], windows: { cellId: string; durationMs: number }[], resources: unknown[], measureOf: (id: string, value: number | null, source: string) => Measure) {
  const own = attempts.filter(a => a.cellId === cellId);
  const counts = zero();
  for (const a of own) counts[a.status === 'skipped-gate' ? 'skipped' : a.status]++;
  const measured = own.filter(a => a.phase === 'measure' && a.status === 'verified');
  const durationMs = windows.filter(w => w.cellId === cellId).reduce((sum, w) => sum + w.durationMs, 0);
  const latency = quantiles(measured.map(a => a.timing.taskMs!).filter(Number.isFinite));
  const failures = counts.failed + counts.timeout + counts.crashed;
  const status = failures ? (counts.verified ? 'mixed' : 'failed') : counts.verified ? 'verified' : counts.unsupported ? 'unsupported' : 'skipped-gate';
  const typed = resources as { peakFootprintBytes?: number | null; peakRssBytes?: number | null; sampledUserCpuMs?: number; sampledSystemCpuMs?: number }[];
  const peak = (key: 'peakFootprintBytes' | 'peakRssBytes') => typed.some(r => typeof r[key] === 'number') ? Math.max(...typed.map(r => r[key] ?? 0)) : null;
  const cpu = typed.some(r => typeof r.sampledUserCpuMs === 'number') ? typed.reduce((sum, r) => sum + (r.sampledUserCpuMs ?? 0) + (r.sampledSystemCpuMs ?? 0), 0) : null;
  const verifiedAll = own.filter(a => a.status === 'verified').length;
  return { cellId, factors, status: status as AttemptStatus | 'mixed', errorCode: null, counts, metrics: {
    'verified-tasks-per-second': measureOf('verified-tasks-per-second', durationMs ? measured.length * 1000 / durationMs : null, 'engine; verified measured tasks over the measured window, rejections in the denominator window'),
    'task-p50-ms-verified': measureOf('task-p50-ms-verified', latency.p50, 'engine; run RPC only'),
    'task-p95-ms-verified': measureOf('task-p95-ms-verified', latency.p95, 'engine; run RPC only, nearest rank'),
    'cpu-ms-per-verified-job': measureOf('cpu-ms-per-verified-job', cpu !== null && verifiedAll ? cpu / verifiedAll : null, 'engine; adapter process tree over all phases'),
    'footprint-peak-bytes': measureOf('footprint-peak-bytes', peak('peakFootprintBytes'), 'engine; adapter process tree'),
    'rss-peak-bytes': measureOf('rss-peak-bytes', peak('peakRssBytes'), 'engine; adapter process tree, shared pages double-counted')
  } };
}
