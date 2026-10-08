// Single entry point of the decision-driven lab: `pnpm lab <command>`.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { labRoot } from '../src/registry.ts';
import { loadRegistry, checkRegistry } from '../src/core/decisions.ts';
import { importLegacy, writeImported } from './import-legacy.ts';
import { buildLedger, findCitations, type Classification } from './ledger.ts';
import { runEngine } from '../src/core/engine.ts';
import { fromLegacyBench, type BenchCampaign } from '../src/core/campaign.ts';
import { assessHost, loadHostClasses, snapshotHost, hostClasses, type HostClass } from '../src/core/host.ts';
import { captureProvenance } from '../src/core/provenance.ts';
import type { Campaign } from '../src/campaign.ts';

const repositoryRoot = join(labRoot, '..');
const ledgerDir = join(labRoot, 'ledger');

async function importCommand() {
  const registry = loadRegistry();
  const classification = JSON.parse(await readFile(join(ledgerDir, 'classification.json'), 'utf8')) as Classification;
  const imported = await importLegacy();
  const citations = await findCitations(repositoryRoot, imported.map(run => run.runId));
  const { errors, ledger, runs } = buildLedger({ runs: imported, classification, citations, decisions: registry.decisions.map(decision => decision.id) });
  if (errors.length) return fail(errors);
  await writeImported(runs, join(ledgerDir, 'imported'));
  await mkdir(ledgerDir, { recursive: true });
  await writeFile(join(ledgerDir, 'results-ledger.json'), JSON.stringify(ledger, null, 1) + '\n');
  const byRole = Object.entries(Object.groupBy(ledger.entries, entry => entry.role)).map(([role, list]) => `${role} ${list!.length}`).join(', ');
  process.stdout.write(`imported ${runs.length} runs (${byRole}); integrity mismatches: ${ledger.entries.filter(entry => entry.integrity === 'mismatch').length}\n`);
}

function validateCommand() {
  const errors = checkRegistry(loadRegistry());
  if (errors.length) return fail(errors);
  process.stdout.write('registry, claims and metric catalog are valid\n');
}

async function ledgerCommand() {
  const ledger = JSON.parse(await readFile(join(ledgerDir, 'results-ledger.json'), 'utf8')) as { entries: { runId: string; role: string; status: string; validity: { scope: string; class: string }[]; rerun: { required: boolean; priority: string | null } }[] };
  for (const entry of ledger.entries) {
    const validity = entry.validity.map(item => `${item.scope}:${item.class}`).join(' ');
    process.stdout.write(`${entry.role.padEnd(12)} ${entry.status.padEnd(18)} ${(entry.rerun.required ? entry.rerun.priority ?? 'yes' : '-').padEnd(4)} ${entry.runId}  ${validity}\n`);
  }
}

async function runCommand() {
  const path = process.argv[3];
  if (!path) return fail(['usage: pnpm lab run <campaign.json>']);
  const raw = JSON.parse(await readFile(path, 'utf8')) as { schemaVersion?: number };
  // Legacy bench campaigns are converted; they request local-dev and may not claim a controlled host.
  const campaign = raw.schemaVersion === 2 ? raw as BenchCampaign : fromLegacyBench(raw as unknown as Campaign);
  const { directory, run } = await runEngine(campaign, { source: path });
  process.stdout.write(`${run.status} ${run.host.classGranted} verified=${run.totals.verified} failed=${run.totals.failed} timeout=${run.totals.timeout} aborted=${run.totals.aborted} skipped=${run.totals.skipped}\n${directory}\n`);
  if (run.status !== 'complete' || run.totals.failed + run.totals.timeout + run.totals.crashed > 0) process.exitCode = 1;
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

const commands: Record<string, () => unknown> = { validate: validateCommand, import: importCommand, ledger: ledgerCommand, run: runCommand, preflight: preflightCommand };
const command = commands[process.argv[2] ?? ''];
if (!command) { process.stderr.write(`usage: pnpm lab <${Object.keys(commands).join('|')}>\n`); process.exitCode = 2; }
else await command();
