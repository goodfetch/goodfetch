// Re-express legacy lab results (schemaVersion 1 and lane-specific shapes) as run.v2 documents.
// Read-only on results/ and reports/: originals are never modified, and output is deterministic.
import { readFile, readdir, mkdir, writeFile, access } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { hashFile } from '../src/environment.ts';
import { labRoot } from '../src/registry.ts';

type Counts = { verified: number; failed: number; timeout: number; crashed: number; aborted: number; unsupported: number; rejected: number; unavailable: number; skipped: number };
type CellStatus = 'verified' | 'failed' | 'mixed' | 'timeout' | 'crashed' | 'aborted' | 'unsupported' | 'rejected' | 'unavailable' | 'skipped-gate';
type Measure = { value: number | null; unit: string; source: string };
export type ImportedCell = { cellId: string; factors: Record<string, string | number | boolean | null>; status: CellStatus; errorCode: string | null; counts: Counts; metrics: Record<string, Measure> };
export type ImportedRun = {
  schemaVersion: 2; runId: string; lane: Lane; campaign: { id: string; source: string | null }; startedAt: string; decisionIds: string[];
  provenance: { git: { available: boolean; sha: string | null; dirty: boolean | null }; sourceTreeSha256: string | null; sourceChanges: number | null; binaryChanges: number | null; toolchains: Record<string, string | null>; integrity: { status: 'verified' | 'mismatch' | 'uncatalogued'; checked: number; mismatches: string[] } };
  host: { classRequested: null; classGranted: 'local-dev'; os: string | null; arch: string | null; cpu: string | null; logicalCpus: number | null; memoryBytes: number | null; preflight: null; postflight: null };
  status: 'complete' | 'invalid-provenance' | 'interrupted'; countUnit: 'attempt' | 'cell' | 'step'; cells: ImportedCell[]; totals: Counts;
  metricAvailability: Record<string, { available: boolean; reason?: string }>;
  legacy: { shape: 'bench' | 'cells' | 'suite' | 'interrupted'; sourceDir: string; errorCodeInferred: boolean; note: string | null };
};
type Lane = 'bench' | 'storage' | 'workflow' | 'runtime' | 'actionability' | 'sqlite-readers' | 'suite';
type Json = Record<string, unknown>;

const runName = /^(?<campaign>.+)-(?<stamp>\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z)-[0-9a-f]{6}$/;
const lanes: [string, Lane][] = [['qualification-integrated', 'suite'], ['qualification-current', 'suite'], ['browser-actionability', 'actionability'], ['sqlite-readers', 'sqlite-readers'], ['storage-', 'storage'], ['workflow-', 'workflow'], ['runtime-', 'runtime']];
const factorKeys = ['backend', 'scenario', 'engine', 'format', 'batchSize', 'batchRows', 'writers', 'readerMode', 'payloadBytes', 'concurrency', 'batchJobs', 'fault', 'poolWake', 'runtime', 'variant', 'policy', 'jobs', 'pages', 'delayMs', 'leaseMs', 'block'];

const zero = (): Counts => ({ verified: 0, failed: 0, timeout: 0, crashed: 0, aborted: 0, unsupported: 0, rejected: 0, unavailable: 0, skipped: 0 });
const add = (into: Counts, from: Partial<Counts>) => { for (const key of Object.keys(into) as (keyof Counts)[]) into[key] += Number(from[key] ?? 0); return into; };
const num = (value: unknown) => typeof value === 'number' && Number.isFinite(value) ? value : null;
const str = (value: unknown) => typeof value === 'string' ? value : null;
const exists = (path: string) => access(path).then(() => true, () => false);
const readJson = async (path: string) => JSON.parse(await readFile(path, 'utf8')) as Json;
const measure = (value: unknown, unit: string, source: string): Measure => ({ value: num(value), unit, source });
const cellId = (factors: Record<string, unknown>, index: number) => `${createHash('sha256').update(JSON.stringify(factors)).digest('hex').slice(0, 12)}-${index}`;

function factorsOf(raw: Json): Record<string, string | number | boolean | null> {
  const source = { ...(typeof raw.config === 'object' && raw.config ? raw.config as Json : {}), ...raw };
  const factors: Record<string, string | number | boolean | null> = {};
  for (const key of factorKeys) { const value = source[key]; if (['string', 'number', 'boolean'].includes(typeof value)) factors[key] = value as string | number | boolean; }
  return factors;
}

function benchCell(row: Json, index: number): ImportedCell {
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

const legacyStatus: Record<string, CellStatus> = { verified: 'verified', failed: 'failed', unsupported: 'unsupported', timeout: 'timeout', crashed: 'crashed', rejected: 'rejected', 'startup-failed': 'crashed' };

function laneMetrics(lane: Lane, raw: Json): Record<string, Measure> {
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

function cellsCell(lane: Lane, raw: Json, index: number): { cell: ImportedCell; inferred: boolean } {
  const original = str(raw.status) ?? 'failed';
  const status = legacyStatus[original] ?? 'failed';
  const counts = zero();
  const bucket: keyof Counts = status === 'skipped-gate' ? 'skipped' : status === 'mixed' ? 'failed' : status;
  counts[bucket] = 1;
  const factors = factorsOf(raw);
  const inferred = original === 'startup-failed';
  return { cell: { cellId: cellId(factors, index), factors, status, errorCode: inferred ? 'E_STARTUP' : null, counts, metrics: laneMetrics(lane, raw) }, inferred };
}

function suiteCell(step: Json, index: number): ImportedCell {
  const verified = num(step.verified) ?? 0, total = num(step.total) ?? 0;
  const status: CellStatus = total > 0 && verified === total ? 'verified' : verified > 0 ? 'mixed' : 'failed';
  const counts = zero(); counts[status === 'verified' ? 'verified' : 'failed'] = 1;
  const factors = { step: str(step.name) };
  return { cellId: cellId(factors, index), factors, status, errorCode: null, counts, metrics: {} };
}

type CatalogEntry = { file: string; sha256: string; copy: string | null };
async function catalogEntries(root: string): Promise<Map<string, CatalogEntry[]>> {
  const reports = join(root, 'reports'), byRun = new Map<string, CatalogEntry[]>();
  const catalogs: { path: string; base: string }[] = [];
  if (await exists(reports)) for (const entry of (await readdir(reports, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
    if (entry.isDirectory() && entry.name.startsWith('evidence') && await exists(join(reports, entry.name, 'catalog.json'))) catalogs.push({ path: join(reports, entry.name, 'catalog.json'), base: reports });
    if (entry.isFile() && entry.name === 'integrated-evidence-catalog.json') catalogs.push({ path: join(reports, entry.name), base: root });
  }
  for (const catalog of catalogs) {
    const files = ((await readJson(catalog.path)).files ?? []) as { path: string; sha256: string }[];
    for (const file of files) {
      const parts = file.path.split('/');
      const runIndex = parts.findIndex(part => runName.test(part));
      if (runIndex < 0) continue;
      const run = parts[runIndex]!, rest = parts.slice(runIndex + 1).join('/');
      const isCopy = parts[0] !== 'results';
      const list = byRun.get(run) ?? []; list.push({ file: rest, sha256: file.sha256, copy: isCopy ? join(catalog.base, file.path) : null }); byRun.set(run, list);
    }
  }
  return byRun;
}

async function integrity(root: string, run: string, entries: CatalogEntry[] | undefined) {
  if (!entries?.length) return { status: 'uncatalogued' as const, checked: 0, mismatches: [] };
  const mismatches: string[] = [];
  for (const entry of entries) {
    const original = join(root, 'results', run, entry.file);
    const checks = [original, ...(entry.copy ? [entry.copy] : [])];
    for (const path of checks) if (!(await exists(path)) || await hashFile(path) !== entry.sha256) { mismatches.push(entry.file); break; }
  }
  return { status: mismatches.length ? 'mismatch' as const : 'verified' as const, checked: entries.length, mismatches: [...new Set(mismatches)].sort() };
}

export async function importLegacy({ root = labRoot }: { root?: string } = {}): Promise<ImportedRun[]> {
  const resultsDir = join(root, 'results');
  const catalog = await catalogEntries(root);
  const names = (await readdir(resultsDir, { withFileTypes: true })).filter(entry => entry.isDirectory() && runName.test(entry.name)).map(entry => entry.name);
  names.sort((a, b) => stamp(a).localeCompare(stamp(b)) || a.localeCompare(b));
  const runs: ImportedRun[] = [];
  for (const name of names) {
    const directory = join(resultsDir, name), match = runName.exec(name)!.groups!;
    const summary = await exists(join(directory, 'summary.json')) ? await readJson(join(directory, 'summary.json')) : null;
    const manifest = await exists(join(directory, 'manifest.json')) ? await readJson(join(directory, 'manifest.json')) : {};
    const interrupted = await exists(join(directory, 'interrupted.json')) ? await readJson(join(directory, 'interrupted.json')) : null;
    const lane: Lane = lanes.find(([prefix]) => name.startsWith(prefix))?.[1] ?? 'bench';
    let shape: ImportedRun['legacy']['shape'], cells: ImportedCell[] = [], inferred = false, countUnit: ImportedRun['countUnit'];
    if (!summary) { shape = 'interrupted'; countUnit = 'step'; }
    else if (Array.isArray(summary.summaries)) { shape = 'bench'; countUnit = 'attempt'; cells = (summary.summaries as Json[]).map(benchCell); }
    else if (Array.isArray(summary.steps)) { shape = 'suite'; countUnit = 'step'; cells = (summary.steps as Json[]).map(suiteCell); }
    else { shape = 'cells'; countUnit = 'cell'; cells = ((summary.cells ?? []) as Json[]).map((raw, index) => { const result = cellsCell(lane, raw, index); inferred ||= result.inferred; return result.cell; }); }
    const sourceChanges = summary && Array.isArray(summary.sourceChanges) ? summary.sourceChanges.length : null;
    const binaryChanges = summary ? Array.isArray(summary.binaryChanges) ? summary.binaryChanges.length : typeof summary.binaryChanged === 'boolean' ? Number(summary.binaryChanged) : null : null;
    const environment = (manifest.environment ?? {}) as Json;
    const sourceHashes = environment.sourceHashes as Record<string, string> | undefined;
    const status: ImportedRun['status'] = interrupted || !summary ? 'interrupted' : (sourceChanges ?? 0) > 0 || (binaryChanges ?? 0) > 0 ? 'invalid-provenance' : 'complete';
    const campaignPath = join(root, 'campaigns', `${match.campaign}.json`);
    runs.push({
      schemaVersion: 2, runId: name, lane, campaign: { id: match.campaign!, source: await exists(campaignPath) ? `campaigns/${match.campaign}.json` : null },
      startedAt: stamp(name), decisionIds: [],
      provenance: {
        git: { available: false, sha: null, dirty: null },
        sourceTreeSha256: sourceHashes ? createHash('sha256').update(JSON.stringify(Object.entries(sourceHashes).sort(([a], [b]) => a.localeCompare(b)))).digest('hex') : null,
        sourceChanges, binaryChanges,
        toolchains: { node: str(environment.node), browser: str((environment.browser as Json | undefined)?.version) },
        integrity: await integrity(root, name, catalog.get(name))
      },
      host: { classRequested: null, classGranted: 'local-dev', os: str(environment.os), arch: str(environment.architecture) ?? str(environment.arch), cpu: str(environment.cpu), logicalCpus: num(environment.logicalCpuCount), memoryBytes: num(environment.totalMemoryBytes), preflight: null, postflight: null },
      status, countUnit, cells, totals: cells.reduce((total, cell) => add(total, cell.counts), zero()),
      metricAvailability: availability(lane, str(environment.os)),
      legacy: { shape, sourceDir: `results/${name}`, errorCodeInferred: inferred, note: interrupted ? str(interrupted.reason) : null }
    });
  }
  return runs;
}

function availability(lane: Lane, os: string | null): ImportedRun['metricAvailability'] {
  const linuxOnly = { available: false, reason: `linux-only probe; legacy run on ${os ?? 'unknown OS'}` };
  return {
    'pss-peak-bytes': linuxOnly, 'cgroup-memory-peak-bytes': linuxOnly,
    'cpu-ms-per-verified-job': lane === 'runtime' ? { available: true } : { available: false, reason: 'CPU was not instrumented in this legacy lane' },
    'footprint-peak-bytes': lane === 'runtime' || lane === 'sqlite-readers' ? { available: true } : { available: false, reason: 'footprint was not instrumented in this legacy lane' }
  };
}

function stamp(name: string) {
  const value = runName.exec(name)?.groups?.stamp ?? '';
  return value.replace(/^(\d{4}-\d{2}-\d{2}T)(\d{2})-(\d{2})-(\d{2})-(\d{3})Z$/, '$1$2:$3:$4.$5Z');
}

export async function writeImported(runs: ImportedRun[], outDir: string) {
  for (const run of runs) {
    await mkdir(join(outDir, run.runId), { recursive: true });
    await writeFile(join(outDir, run.runId, 'run.v2.json'), JSON.stringify(run, null, 1) + '\n');
  }
}
