// Re-express legacy lab results (schemaVersion 1 and lane-specific shapes) as run.v2 documents.
// Read-only on results/ and reports/: originals are never modified, and output is deterministic.
import { copyFile, readFile, readdir, mkdir, writeFile, access } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { hashFile } from '../src/environment.ts';
import { labRoot } from '../src/registry.ts';
import { zero, add, num, str, benchCell, cellsCell, suiteCell, type Counts, type ImportedCell, type Lane, type Json } from '../src/core/legacy-cells.ts';

export type ImportedRun = {
  schemaVersion: 2; runId: string; lane: Lane; campaign: { id: string; source: string | null }; startedAt: string; decisionIds: string[];
  provenance: { git: { available: boolean; sha: string | null; dirty: boolean | null }; sourceTreeSha256: string | null; sourceChanges: number | null; binaryChanges: number | null; toolchains: Record<string, string | null>; integrity: { status: 'verified' | 'mismatch' | 'uncatalogued'; checked: number; mismatches: string[] } };
  host: { classRequested: string | null; classGranted: 'local-dev' | 'local-controlled' | 'reference'; os: string | null; arch: string | null; cpu: string | null; logicalCpus: number | null; memoryBytes: number | null; preflight: null; postflight: null };
  status: 'complete' | 'invalid-provenance' | 'interrupted' | 'noisy-host' | 'aborted'; countUnit: 'attempt' | 'cell' | 'step'; cells: ImportedCell[]; totals: Counts;
  metricAvailability: Record<string, { available: boolean; reason?: string }>;
  legacy: { shape: 'bench' | 'cells' | 'suite' | 'interrupted'; sourceDir: string; errorCodeInferred: boolean; note: string | null } | null;
};

const runName = /^(?<campaign>.+)-(?<stamp>\d{4}-\d{2}-\d{2}T\d{2}-\d{2}-\d{2}-\d{3}Z)-[0-9a-f]{6}$/;
const lanes: [string, Lane][] = [['qualification-integrated', 'suite'], ['qualification-current', 'suite'], ['browser-actionability', 'actionability'], ['sqlite-readers', 'sqlite-readers'], ['storage-', 'storage'], ['workflow-', 'workflow'], ['runtime-', 'runtime']];
const exists = (path: string) => access(path).then(() => true, () => false);
const readJson = async (path: string) => JSON.parse(await readFile(path, 'utf8')) as Json;

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
  const names: string[] = [];
  for (const entry of await readdir(resultsDir, { withFileTypes: true })) if (entry.isDirectory() && runName.test(entry.name) && !(await exists(join(resultsDir, entry.name, 'run.v2.json')))) names.push(entry.name);
  names.sort((a, b) => stamp(a).localeCompare(stamp(b)) || a.localeCompare(b));
  const runs: ImportedRun[] = [];
  for (const name of names) {
    const directory = join(resultsDir, name), match = runName.exec(name)!.groups!;
    const summary = await exists(join(directory, 'summary.json')) ? await readJson(join(directory, 'summary.json')) : null;
    const manifest = await exists(join(directory, 'manifest.json')) ? await readJson(join(directory, 'manifest.json')) : {};
    const interrupted = await exists(join(directory, 'interrupted.json')) ? await readJson(join(directory, 'interrupted.json')) : null;
    const lane: Lane = lanes.find(([prefix]) => name.startsWith(prefix))?.[1] ?? 'bench';
    let shape: NonNullable<ImportedRun['legacy']>['shape'], cells: ImportedCell[] = [], inferred = false, countUnit: ImportedRun['countUnit'];
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

// Native v2 runs are read as written by the engine; they are never rewritten. The ledger keeps a byte-identical copy of each
// (ledger/runs), committed with the repository, so a clone without results/ still holds every run a verdict uses.
export async function loadV2Runs({ root = labRoot }: { root?: string } = {}): Promise<ImportedRun[]> {
  const runs = new Map<string, ImportedRun>();
  for (const directory of [join(root, 'results'), join(root, 'ledger/runs')]) {
    if (!(await exists(directory))) continue;
    for (const entry of (await readdir(directory, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      const path = join(directory, entry.name, 'run.v2.json');
      if (entry.isDirectory() && !runs.has(entry.name) && await exists(path)) runs.set(entry.name, await readJson(path) as unknown as ImportedRun);
    }
  }
  return [...runs.values()];
}
export async function copyV2Runs(runIds: string[], { root = labRoot }: { root?: string } = {}) {
  for (const runId of runIds) {
    const source = join(root, 'results', runId, 'run.v2.json'); if (!(await exists(source))) continue;
    await mkdir(join(root, 'ledger/runs', runId), { recursive: true }); await copyFile(source, join(root, 'ledger/runs', runId, 'run.v2.json'));
  }
}
