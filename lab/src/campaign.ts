import { mkdir, writeFile, appendFile } from 'node:fs/promises';
import { join } from 'node:path';
import { randomBytes } from 'node:crypto';
import { startFixtures, type RunSpec, type Scenario } from '../fixtures/server.ts';
import { WorkerClient, type WorkerCommand } from './worker-client.ts';
import { commandFor, backendNames, labRoot, type BackendName } from './registry.ts';
import { summarize, shuffle, quantiles, type TerminalStatus } from './statistics.ts';
import { schedule } from './scheduler.ts';
import { environment, sourceFingerprint } from './environment.ts';
import { sampleProcessGroup } from './resources.ts';
import type { AdapterOutput } from './worker-server.ts';
import { writeReport } from './report.ts';

export type Campaign = {
  name: string; backends: BackendName[]; scenarios: Scenario[]; seed: number; blocks: number; iterations: number;
  warmup: number; concurrency: number; mode: 'closed' | 'open'; ratePerSecond?: number; timeoutMs: number; maxDurationMs: number;
  pages: number; actions: number; delayMs?: number; variants?: NonNullable<RunSpec['variant']>[];
  poolWake?:'poll'|'notify';
};
export type Row = { backend: string; scenario: string; block: number; phase: 'conformance' | 'warmup' | 'measure'; index: number; seed: number;
  status: TerminalStatus; elapsedMs: number | null; queueMs: number; reasons: string[]; metrics?: Record<string, number>; requests?: number; effects?: number };
export function validateCampaign(value: unknown): asserts value is Campaign {
  const c = value as Campaign;
  if (!c || typeof c.name !== 'string' || !/^[a-z0-9-]{1,80}$/.test(c.name)) throw new Error('invalid campaign name');
  if (!Array.isArray(c.backends) || !c.backends.length || !c.backends.every(b => backendNames.includes(b)) || new Set(c.backends).size !== c.backends.length) throw new Error('invalid backends');
  const scenarios = ['http-catalog', 'js-catalog', 'form', 'clicks', 'shadow', 'iframe', 'download','actionability','primitives','navigation'];
  if (!Array.isArray(c.scenarios) || !c.scenarios.length || !c.scenarios.every(s => scenarios.includes(s))) throw new Error('invalid scenarios');
  for (const [key, min, max] of [['seed',0,2**32-1], ['blocks',1,100], ['iterations',1,100_000], ['warmup',0,1000], ['concurrency',1,128], ['timeoutMs',50,120_000], ['maxDurationMs',100,3_600_000], ['pages',1,5000], ['actions',1,1000]] as const) {
    if (!Number.isInteger(c[key]) || c[key] < min || c[key] > max) throw new Error(`invalid ${key}`);
  }
  if (!['closed', 'open'].includes(c.mode) || (c.mode === 'open' && (!Number.isFinite(c.ratePerSecond) || c.ratePerSecond! <= 0 || c.ratePerSecond! > 10000))) throw new Error('invalid arrival model');
  if (c.delayMs !== undefined && (!Number.isInteger(c.delayMs) || c.delayMs < 0 || c.delayMs > 1000)) throw new Error('invalid fixture delay');
  if(c.poolWake!==undefined&&!['poll','notify'].includes(c.poolWake))throw new Error('invalid pool wake mode');
  if (c.variants !== undefined && (!Array.isArray(c.variants) || !c.variants.length || !c.variants.every(v => ['normal','overlay','rerender','delayed','hidden','moving','never-enabled','never-uncovered','ambiguous','http-redirect','http-gzip','http-chunked','http-large','hover','dblclick','contextmenu','drag','wheel','keyboard','contenteditable','unicode','moved','retarget','spa','history','popup','frame','cross-frame','upload','download','http-429','http-503-once','http-truncated','http-reset','http-redirect-loop','http-slow','robots-disallow','challenge-200','tls-fingerprint'].includes(v)))) throw new Error('invalid variants');
  const planned = c.backends.length * c.scenarios.length * c.blocks * (c.iterations + c.warmup + (c.variants?.length ?? 1));
  if (planned > 200000 || planned * c.pages * 2 > 1000000) throw new Error('fixture corpus budget exceeded; split campaign or use a streaming oracle');
}
function output(value: unknown): AdapterOutput {
  const out = value as AdapterOutput;
  if (!out || !['ok','unsupported'].includes(out.status)) throw new Error('invalid adapter output');
  if (out.records && (!Array.isArray(out.records) || out.records.some(r => typeof r.id !== 'string' || !Number.isFinite(r.price)))) throw new Error('invalid records');
  if (out.file !== undefined && typeof out.file !== 'string') throw new Error('invalid downloaded bytes');
  if (out.metrics !== undefined && (typeof out.metrics !== 'object' || out.metrics === null || Object.values(out.metrics).some(value => typeof value !== 'number' || !Number.isFinite(value) || value < 0))) throw new Error('invalid metrics');
  return out;
}
export async function runCampaign(config: Campaign, options: { outputRoot?: string; command?: (backend: BackendName) => WorkerCommand; skipEnvironment?: boolean } = {}) {
  validateCampaign(config);
  const id = `${config.name}-${new Date().toISOString().replace(/[:.]/g,'-')}-${randomBytes(3).toString('hex')}`;
  const directory = join(options.outputRoot ?? join(labRoot, 'results'), id); await mkdir(directory, { recursive: true });
  const env = options.skipEnvironment ? null : await environment();
  const fixtures = await startFixtures(); const rows: Row[] = []; const diagnostics: unknown[] = [];
  const measurementWindows: { backend: string; scenario: string; block: number; durationMs: number }[] = [];
  const manifest = { schemaVersion: 1, id, startedAt: new Date().toISOString(), config, environment: env, fixtureOrigin: fixtures.origin,
    methodology: { monotonicClock: true, timing: 'worker startup separately; run IPC + task + independent oracle included', warmupExcludedFromSummary: true, conformanceGate: true, blockedTrialsRandomized: true,
      resourceLimits: 'bounded concurrency, per-call timeout and per-cell scheduling deadline; no OS hard CPU/RAM limit on macOS', network: 'loopback; no provider calls requested', quality: 'private fixture oracle' } };
  let writes = Promise.resolve();
  async function record(row: Row) { rows.push(row); writes = writes.then(() => appendFile(join(directory, 'attempts.jsonl'), JSON.stringify(row) + '\n')); await writes; }
  try {
    await writeFile(join(directory, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n');
    for (let block = 0; block < config.blocks; block++) {
      for (const backend of shuffle(config.backends, config.seed + block)) {
        const worker = new WorkerClient(options.command?.(backend) ?? commandFor(backend, config.timeoutMs, env?.browser.executablePath));
        const resources = sampleProcessGroup(worker.pid); const startup = performance.now();
        let capabilities: string[] = []; let startupError: string | null = null; let info: unknown;
        try {
          info = await worker.call('hello', {}); capabilities = (info as { capabilities: string[] }).capabilities;
          if (!Array.isArray(capabilities) || !capabilities.every(c => typeof c === 'string')) throw new Error('invalid capabilities');
          if (config.concurrency > ((info as { maxConcurrency?: number }).maxConcurrency ?? 128)) throw new Error('campaign exceeds adapter concurrency contract');
          await worker.call('prepare', { executablePath: env?.browser.executablePath, headless: true, concurrency: config.concurrency,poolWake:config.poolWake }, Math.max(config.timeoutMs, 45_000));
        } catch (error) { startupError = error instanceof Error ? error.message : String(error); }
        const startupMs = performance.now() - startup;
        const experimentStart = performance.now();
        async function attempt(scenario: Scenario, phase: Row['phase'], index: number, queueMs = 0) {
          const seed = config.seed + block * 1_000_003 + index + (phase === 'measure' ? 10000 : phase === 'warmup' ? 1000 : 0); const began = performance.now();
          if (startupError) { await record({ backend, scenario, block, phase, index, seed, queueMs, status: 'crashed', elapsedMs: null, reasons: [startupError] }); return; }
          if (!capabilities.includes(scenario)) { await record({ backend, scenario, block, phase, index, seed, queueMs, status: 'unsupported', elapsedMs: null, reasons: ['capability not advertised'] }); return; }
          const variant = config.variants?.[index % config.variants.length] ?? 'normal';
          const fixture = await fixtures.createRun({ scenario, seed, pages: config.pages, actions: config.actions, delayMs: config.delayMs, variant });
          try {
            const out = output(await worker.call('run', { ...config, scenario, seed, variant, url: fixture.url }));
            const verdict = out.status === 'unsupported' ? { status: 'unsupported' as const, reasons: [out.reason ?? 'unsupported'] } : await fixtures.verify(fixture.id, out);
            await record({ backend, scenario, block, phase, index, seed, queueMs, status: verdict.status, elapsedMs: performance.now() - began, reasons: verdict.reasons, metrics: out.metrics, ...fixtures.snapshot(fixture.id) });
          } catch (error) {
            const message = error instanceof Error ? error.message : String(error);
            await record({ backend, scenario, block, phase, index, seed, queueMs, status: message.includes('timeout') ? 'timeout' : message.includes('closed') || message.includes('exited') ? 'crashed' : 'failed', elapsedMs: performance.now() - began, reasons: [message], ...fixtures.snapshot(fixture.id) });
          }
        }
        try {
          for (const scenario of config.scenarios) {
            let qualified = true;
            for (let i = 0; i < (config.variants?.length ?? 1); i++) { await attempt(scenario, 'conformance', i); if (rows[rows.length - 1]!.status !== 'verified') { qualified = false; break; } }
            if (!qualified) continue;
            for (let i = 0; i < config.warmup; i++) { await attempt(scenario, 'warmup', i + 1); if (rows[rows.length - 1]!.status !== 'verified') { qualified = false; break; } }
            if (!qualified) continue;
            const window = await schedule({ ...config, iterations: config.iterations }, (index, queueMs) => attempt(scenario, 'measure', index + config.warmup + 1, queueMs),
              (index, queueMs) => record({ backend, scenario, block, phase: 'measure', index: index + config.warmup + 1, seed: config.seed + block * 1_000_003 + index + config.warmup + 1 + 10000, queueMs, status: 'rejected', elapsedMs: null, reasons: ['arrival exceeds admission or campaign deadline'] }));
            measurementWindows.push({ backend, scenario, block, durationMs: window.durationMs });
          }
        } finally {
          await worker.close(); diagnostics.push({ backend, block, info, startupMs, startupError, experimentMs: performance.now() - experimentStart, resources: resources.stop(), stderr: worker.stderr });
        }
        process.stderr.write(`${backend}: block ${block + 1}/${config.blocks} finished\n`);
      }
    }
  } finally { await fixtures.close(); }
  await writes;
  const summaries = config.backends.flatMap(backend => config.scenarios.map(scenario => {
    const measured = rows.filter(row => row.backend === backend && row.scenario === scenario && row.phase === 'measure');
    const windows = measurementWindows.filter(w => w.backend === backend && w.scenario === scenario);
    const durationMs = windows.reduce((total, w) => total + w.durationMs, 0);
    return { backend, scenario, conformance: summarize(rows.filter(row => row.backend === backend && row.scenario === scenario && row.phase === 'conformance')), warmup: summarize(rows.filter(row => row.backend === backend && row.scenario === scenario && row.phase === 'warmup')),
      measurement: summarize(measured), throughput: { durationMs, verifiedTasksPerSecond: durationMs ? measured.filter(r => r.status === 'verified').length * 1000 / durationMs : null, plannedTasksPerSecond: durationMs ? measured.length * 1000 / durationMs : null, configuredRatePerSecond: config.mode === 'open' ? config.ratePerSecond : null },
      admissionDelayMs: quantiles(measured.map(row => row.queueMs)),
      endToEndMs: quantiles(measured.filter(row => row.elapsedMs !== null).map(row => row.elapsedMs! + row.queueMs)),
      blocks: windows.map(w => ({ block: w.block, durationMs: w.durationMs, measurement: summarize(measured.filter(row => row.block === w.block)) })) };
  }));
  const sourceChanges = env ? Object.entries(await sourceFingerprint()).filter(([path,hash]) => env.sourceHashes[path] !== hash).map(([path]) => path) : [];
  const summary = { id, summaries, diagnostics, sourceChanges, finishedAt: new Date().toISOString(), limitations: ['local exploratory campaign', 'no monetary or token cost measured in deterministic mode', 'RSS is sampled and double-counts shared pages', 'no universal performance ranking', 'backend lifecycle differs; consult diagnostics', 'admissionDelayMs is generator lateness; no FIFO queue', 'zeroFailureUpper95 assumes independent representative Bernoulli trials; this corpus does not establish them', ...(sourceChanges.length ? ['source changed during campaign; rerun before interpreting performance'] : [])] };
  await writeFile(join(directory, 'summary.json'), JSON.stringify(summary, null, 2) + '\n');
  await writeReport(directory, summary);
  return { directory, rows, summary };
}
