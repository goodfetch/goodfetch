// Single entry point of the decision-driven lab: `pnpm lab <command>`.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { join } from 'node:path';
import { labRoot } from '../src/registry.ts';
import { loadRegistry, checkRegistry } from '../src/core/decisions.ts';
import { importLegacy, writeImported } from './import-legacy.ts';
import { buildLedger, findCitations, type Classification } from './ledger.ts';

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

function fail(errors: string[]) { for (const error of errors) process.stderr.write(`${error}\n`); process.exitCode = 1; }

const commands: Record<string, () => unknown> = { validate: validateCommand, import: importCommand, ledger: ledgerCommand };
const command = commands[process.argv[2] ?? ''];
if (!command) { process.stderr.write(`usage: pnpm lab <${Object.keys(commands).join('|')}>\n`); process.exitCode = 2; }
else await command();
