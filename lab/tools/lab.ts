// Single entry point of the decision-driven lab: `pnpm lab <command>`.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { labRoot } from '../src/registry.ts';
import { loadRegistry, checkRegistry } from '../src/core/decisions.ts';
import { importLegacy, writeImported, loadV2Runs } from './import-legacy.ts';
import { buildLedger, findCitations, type Classification } from './ledger.ts';
import { runEngine } from '../src/core/engine.ts';
import { fromLegacyBench, validateBenchCampaign, type BenchCampaign } from '../src/core/campaign.ts';
import { runLaneCampaign, validateLaneCampaign, drivers, type LaneCampaign } from '../src/core/lanes.ts';
import { validateDocument } from '../src/core/schema.ts';
import { readdir } from 'node:fs/promises';
import { assessHost, loadHostClasses, snapshotHost, hostClasses, type HostClass } from '../src/core/host.ts';
import { captureProvenance } from '../src/core/provenance.ts';
import type { Campaign } from '../src/campaign.ts';

const repositoryRoot = join(labRoot, '..');
const ledgerDir = join(labRoot, 'ledger');

async function importCommand() {
  const registry = loadRegistry();
  const classification = JSON.parse(await readFile(join(ledgerDir, 'classification.json'), 'utf8')) as Classification;
  const imported = [...await importLegacy(), ...await loadV2Runs()].sort((a, b) => a.startedAt.localeCompare(b.startedAt) || a.runId.localeCompare(b.runId));
  const citations = await findCitations(repositoryRoot, imported.map(run => run.runId));
  const { errors, ledger, runs } = buildLedger({ runs: imported, classification, citations, decisions: registry.decisions.map(decision => decision.id) });
  if (errors.length) return fail(errors);
  await writeImported(runs.filter(run => run.legacy !== null), join(ledgerDir, 'imported'));
  await mkdir(ledgerDir, { recursive: true });
  await writeFile(join(ledgerDir, 'results-ledger.json'), JSON.stringify(ledger, null, 1) + '\n');
  const byRole = Object.entries(Object.groupBy(ledger.entries, entry => entry.role)).map(([role, list]) => `${role} ${list!.length}`).join(', ');
  process.stdout.write(`imported ${runs.length} runs (${byRole}); integrity mismatches: ${ledger.entries.filter(entry => entry.integrity === 'mismatch').length}\n`);
}

async function validateCommand() {
  const registry = loadRegistry();
  const errors = checkRegistry(registry);
  const decisions = new Set(registry.decisions.map(decision => decision.id));
  const files: string[] = [];
  for (const entry of await readdir(join(labRoot, 'campaigns/v2'), { withFileTypes: true, recursive: true })) if (entry.isFile() && entry.name.endsWith('.json')) files.push(join(entry.parentPath, entry.name));
  for (const file of files.sort()) {
    const name = file.slice(labRoot.length);
    try {
      const raw = JSON.parse(await readFile(file, 'utf8')) as { lane?: string; steps?: { campaign: string }[]; decisionIds?: string[] };
      if (raw.steps) { const problems = validateDocument('suite.v2', raw); if (problems.length) throw new Error(problems.join('; ')); for (const step of raw.steps) await readFile(join(labRoot, step.campaign)); continue; }
      if (raw.lane === 'bench') validateBenchCampaign(raw); else { validateLaneCampaign(raw); drivers[(raw as LaneCampaign).lane].validate((raw as LaneCampaign).config); }
      for (const id of raw.decisionIds ?? []) if (!decisions.has(id)) throw new Error(`unknown decision ${id}`);
    } catch (error) { errors.push(`${name}: ${error instanceof Error ? error.message : String(error)}`); }
  }
  if (errors.length) return fail(errors);
  process.stdout.write(`registry, claims, metric catalog and ${files.length} v2 campaigns are valid\n`);
}

async function ledgerCommand() {
  const ledger = JSON.parse(await readFile(join(ledgerDir, 'results-ledger.json'), 'utf8')) as { entries: { runId: string; role: string; status: string; validity: { scope: string; class: string }[]; rerun: { required: boolean; priority: string | null } }[] };
  for (const entry of ledger.entries) {
    const validity = entry.validity.map(item => `${item.scope}:${item.class}`).join(' ');
    process.stdout.write(`${entry.role.padEnd(12)} ${entry.status.padEnd(18)} ${(entry.rerun.required ? entry.rerun.priority ?? 'yes' : '-').padEnd(4)} ${entry.runId}  ${validity}\n`);
  }
}

async function loadCampaign(path: string): Promise<BenchCampaign | LaneCampaign> {
  const raw = JSON.parse(await readFile(path, 'utf8')) as { schemaVersion?: number; lane?: string };
  // Legacy bench campaigns are converted; they request local-dev and may not claim a controlled host.
  if (raw.schemaVersion !== 2) return fromLegacyBench(raw as unknown as Campaign);
  return raw as BenchCampaign | LaneCampaign;
}
async function execute(path: string) {
  const campaign = await loadCampaign(path);
  return campaign.lane === 'bench' ? runEngine(campaign, { source: path }) : runLaneCampaign(campaign, { source: path });
}
async function runCommand() {
  const path = process.argv[3];
  if (!path) return fail(['usage: pnpm lab run <campaign.json>']);
  const { directory, run } = await execute(path);
  process.stdout.write(`${run.status} ${run.host.classGranted} verified=${run.totals.verified} failed=${run.totals.failed} timeout=${run.totals.timeout} crashed=${run.totals.crashed} aborted=${run.totals.aborted} unsupported=${run.totals.unsupported} skipped=${run.totals.skipped}\n${directory}\n`);
  if (run.status !== 'complete' || run.totals.failed + run.totals.timeout + run.totals.crashed > 0) process.exitCode = 1;
}

// A suite runs campaigns one after another and compares verified counts with the expectations it declares.
async function suiteCommand() {
  const path = process.argv[3];
  if (!path) return fail(['usage: pnpm lab suite <suite.json>']);
  const suite = JSON.parse(await readFile(path, 'utf8')) as { steps: { campaign: string; expected: { verified: number; unsupported?: number; failed?: number } }[] };
  const problems = validateDocument('suite.v2', suite);
  if (problems.length) return fail(problems.map(problem => `${path}: ${problem}`));
  const outcomes: string[] = [];
  for (const step of suite.steps) {
    const { directory, run } = await execute(join(labRoot, step.campaign));
    const expected = { unsupported: 0, failed: 0, ...step.expected };
    const ok = run.totals.verified === expected.verified && run.totals.unsupported === expected.unsupported && run.totals.failed + run.totals.crashed + run.totals.timeout === expected.failed && run.status !== 'invalid-provenance';
    outcomes.push(`${ok ? 'ok  ' : 'FAIL'} ${step.campaign} verified ${run.totals.verified}/${expected.verified} status ${run.status} ${directory}`);
    process.stderr.write(outcomes.at(-1) + '\n');
  }
  process.stdout.write(outcomes.join('\n') + '\n');
  if (outcomes.some(line => line.startsWith('FAIL'))) process.exitCode = 1;
}

async function preflightCommand() {
  const requested = (process.argv[3] ?? 'local-controlled') as HostClass;
  if (!hostClasses.includes(requested)) return fail([`unknown host class ${requested}`]);
  const provenance = await captureProvenance();
  const snapshot = await snapshotHost([process.pid]);
  const assessment = assessHost({ snapshot, requested, allowDowngrade: true, gitClean: Boolean(provenance.git.available && provenance.git.dirty === false), classes: loadHostClasses() });
  for (const gate of assessment.gates) process.stdout.write(`${gate.ok ? 'ok  ' : 'FAIL'} ${gate.class.padEnd(16)} ${gate.id.padEnd(20)} ${String(gate.value).padEnd(22)} ${gate.limit}\n`);
  if (snapshot.foreignTop.length) process.stdout.write(`foreign CPU: ${snapshot.foreignTop.map(p => `${p.command} ${p.cpuPercent}%`).join(', ')}\n`);
  process.stdout.write(`requested ${requested}, granted ${assessment.classGranted}\n`);
  if (assessment.classGranted !== requested) process.exitCode = 1;
}

function fail(errors: string[]) { for (const error of errors) process.stderr.write(`${error}\n`); process.exitCode = 1; }

const commands: Record<string, () => unknown> = { validate: validateCommand, import: importCommand, ledger: ledgerCommand, run: runCommand, suite: suiteCommand, preflight: preflightCommand };
const command = commands[process.argv[2] ?? ''];
if (!command) { process.stderr.write(`usage: pnpm lab <${Object.keys(commands).join('|')}>\n`); process.exitCode = 2; }
else await command();
