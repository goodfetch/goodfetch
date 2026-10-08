// Mapping of lane-specific cell records (legacy summaries and lane drivers) to run.v2 cells.
import { createHash } from 'node:crypto';

export type Counts = { verified: number; failed: number; timeout: number; crashed: number; aborted: number; unsupported: number; rejected: number; unavailable: number; skipped: number };
export type CellStatus = 'verified' | 'failed' | 'mixed' | 'timeout' | 'crashed' | 'aborted' | 'unsupported' | 'rejected' | 'unavailable' | 'skipped-gate';
export type Measure = { value: number | null; unit: string; source: string };
export type ImportedCell = { cellId: string; factors: Record<string, string | number | boolean | null>; status: CellStatus; errorCode: string | null; counts: Counts; metrics: Record<string, Measure> };
export type Lane = 'bench' | 'storage' | 'workflow' | 'runtime' | 'actionability' | 'sqlite-readers' | 'suite';
export type Json = Record<string, unknown>;

const factorKeys = ['backend', 'scenario', 'engine', 'format', 'batchSize', 'batchRows', 'writers', 'readerMode', 'payloadBytes', 'concurrency', 'batchJobs', 'fault', 'poolWake', 'runtime', 'variant', 'policy', 'jobs', 'pages', 'delayMs', 'leaseMs', 'block'];

export const zero = (): Counts => ({ verified: 0, failed: 0, timeout: 0, crashed: 0, aborted: 0, unsupported: 0, rejected: 0, unavailable: 0, skipped: 0 });
export const add = (into: Counts, from: Partial<Counts>) => { for (const key of Object.keys(into) as (keyof Counts)[]) into[key] += Number(from[key] ?? 0); return into; };
export const num = (value: unknown) => typeof value === 'number' && Number.isFinite(value) ? value : null;
export const str = (value: unknown) => typeof value === 'string' ? value : null;
export const measure = (value: unknown, unit: string, source: string): Measure => ({ value: num(value), unit, source });
export const cellId = (factors: Record<string, unknown>, index: number) => `${createHash('sha256').update(JSON.stringify(factors)).digest('hex').slice(0, 12)}-${index}`;

export function factorsOf(raw: Json): Record<string, string | number | boolean | null> {
  const source = { ...(typeof raw.config === 'object' && raw.config ? raw.config as Json : {}), ...raw };
  const factors: Record<string, string | number | boolean | null> = {};
  for (const key of factorKeys) { const value = source[key]; if (['string', 'number', 'boolean'].includes(typeof value)) factors[key] = value as string | number | boolean; }
  return factors;
}

export function benchCell(row: Json, index: number): ImportedCell {
  const counts = zero();
  for (const phase of ['conformance', 'warmup', 'measurement']) add(counts, (row[phase] ?? {}) as Partial<Counts>);
  const failures = counts.failed + counts.timeout + counts.crashed;
  const status: CellStatus = failures ? (counts.verified ? 'mixed' : 'failed') : counts.verified ? 'verified' : counts.unsupported ? 'unsupported' : 'skipped-gate';
  const measurement = (row.measurement ?? {}) as Json, latency = (measurement.successLatencyMs ?? {}) as Json, throughput = (row.throughput ?? {}) as Json;
  const factors = factorsOf(row);
  const source = 'legacy-summary; duration includes run RPC and in-process oracle';
  return { cellId: cellId(factors, index), factors, status, errorCode: null, counts, metrics: {
    'verified-tasks-per-second': measure(throughput.verifiedTasksPerSecond, 'task/s', source),
    'task-p50-ms-verified': measure(latency.p50, 'ms', source), 'task-p95-ms-verified': measure(latency.p95, 'ms', source) } };
}

export const legacyStatus: Record<string, CellStatus> = { verified: 'verified', failed: 'failed', unsupported: 'unsupported', timeout: 'timeout', crashed: 'crashed', rejected: 'rejected', 'startup-failed': 'crashed' };

export function laneMetrics(lane: Lane, raw: Json): Record<string, Measure> {
  const resources = (raw.resources && !Array.isArray(raw.resources) ? raw.resources : {}) as Json;
  const metrics: Record<string, Measure> = {};
  if (lane === 'storage' || lane === 'sqlite-readers') metrics['verified-rows-per-second'] = measure(raw.verifiedRowsPerSecond, 'row/s', 'legacy-summary');
  if (lane === 'workflow' || lane === 'runtime') metrics['verified-jobs-per-second'] = measure(raw.verifiedJobsPerSecond, 'job/s', 'legacy-summary; work plus finalization');
  if (lane === 'actionability') metrics['task-ms'] = measure(raw.taskMs, 'ms', 'legacy-summary; startup separated');
  if (num(resources.peakRssBytes) !== null) metrics['rss-peak-bytes'] = measure(resources.peakRssBytes, 'B', `legacy-summary; ${str(resources.method) ?? 'sampled RSS'}`);
  if (num(resources.peakFootprintBytes) !== null) metrics['footprint-peak-bytes'] = measure(resources.peakFootprintBytes, 'B', 'legacy-summary; proc_pid_rusage V4');
  const cpu = num(resources.sampledUserCpuMs), system = num(resources.sampledSystemCpuMs), jobs = num(raw.verifiedJobs);
  if (lane === 'runtime' && cpu !== null && system !== null && jobs) metrics['cpu-ms-per-verified-job'] = measure((cpu + system) / jobs, 'ms/job', 'legacy-summary; sampled proc_pid_rusage V4 over the coordinator tree');
  return metrics;
}

export function cellsCell(lane: Lane, raw: Json, index: number): { cell: ImportedCell; inferred: boolean } {
  const original = str(raw.status) ?? 'failed';
  const status = legacyStatus[original] ?? 'failed';
  const counts = zero();
  const bucket: keyof Counts = status === 'skipped-gate' ? 'skipped' : status === 'mixed' ? 'failed' : status;
  counts[bucket] = 1;
  const factors = factorsOf(raw);
  const inferred = original === 'startup-failed';
  return { cell: { cellId: cellId(factors, index), factors, status, errorCode: inferred ? 'E_STARTUP' : null, counts, metrics: laneMetrics(lane, raw) }, inferred };
}

export function suiteCell(step: Json, index: number): ImportedCell {
  const verified = num(step.verified) ?? 0, total = num(step.total) ?? 0;
  const status: CellStatus = total > 0 && verified === total ? 'verified' : verified > 0 ? 'mixed' : 'failed';
  const counts = zero(); counts[status === 'verified' ? 'verified' : 'failed'] = 1;
  const factors = { step: str(step.name) };
  return { cellId: cellId(factors, index), factors, status, errorCode: null, counts, metrics: {} };
}

