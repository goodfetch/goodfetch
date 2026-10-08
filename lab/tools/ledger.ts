// Build the results ledger: mechanical facts from imported runs, human judgement from ledger/classification.json.
import { readFile, readdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join, relative } from 'node:path';
import type { ImportedRun } from './import-legacy.ts';
import { validateDocument } from '../src/core/schema.ts';

export type ClassificationEntry = {
  role: 'selected' | 'superseded' | 'diagnostic' | 'smoke' | 'orchestrator'; reason: string; supersededBy: string | null; decisionIds: string[];
  validity: { scope: string; class: 'A' | 'B' | 'C' | 'none'; note: string }[]; noisyHost: boolean;
  rerun: { required: boolean; priority: 'P0' | 'P1' | 'P2' | null; hostClass: 'local-controlled' | 'reference' | null };
};
export type Classification = { schemaVersion: 1; runs: Record<string, ClassificationEntry> };
export type LedgerEntry = {
  runId: string; lane: string; campaign: string; startedAt: string; status: ImportedRun['status'] | 'noisy-host'; hostClass: string;
  role: ClassificationEntry['role']; supersededBy: string | null; reason: string; validity: ClassificationEntry['validity']; decisionIds: string[];
  citedIn: string[]; integrity: ImportedRun['provenance']['integrity']['status']; rerun: ClassificationEntry['rerun']; imported: string;
};
export type Ledger = { schemaVersion: 1; classificationSha256: string; entries: LedgerEntry[] };

export function buildLedger({ runs, classification, citations, decisions }: { runs: ImportedRun[]; classification: Classification; citations: Record<string, string[]>; decisions: string[] }) {
  const errors: string[] = [];
  for (const problem of validateDocument('classification', classification)) errors.push(`classification: ${problem}`);
  const runIds = new Set(runs.map(run => run.runId));
  for (const id of Object.keys(classification.runs)) if (!runIds.has(id)) errors.push(`${id}: classified but not a result run`);
  const merged: ImportedRun[] = [], entries: LedgerEntry[] = [];
  for (const run of runs) {
    const entry = classification.runs[run.runId];
    if (!entry) { errors.push(`${run.runId}: no classification`); continue; }
    const cited = [...new Set(citations[run.runId] ?? [])].sort();
    if ((entry.role === 'selected' || entry.role === 'orchestrator') && !cited.length) errors.push(`${run.runId}: role ${entry.role} but not cited in any report`);
    if (entry.role === 'superseded' && (!entry.supersededBy || !runIds.has(entry.supersededBy))) errors.push(`${run.runId}: superseded by ${entry.supersededBy ?? 'nothing'}, which is not a result run`);
    for (const decision of entry.decisionIds) if (!decisions.includes(decision)) errors.push(`${run.runId}: unknown decision ${decision}`);
    if (entry.role === 'selected' && !entry.validity.length) errors.push(`${run.runId}: selected without a validity class`);
    const status = entry.noisyHost && run.status === 'complete' ? 'noisy-host' : run.status;
    const updated: ImportedRun = { ...run, decisionIds: [...entry.decisionIds], status: status as ImportedRun['status'] };
    merged.push(updated);
    entries.push({ runId: run.runId, lane: run.lane, campaign: run.campaign.id, startedAt: run.startedAt, status, hostClass: run.host.classGranted,
      role: entry.role, supersededBy: entry.supersededBy, reason: entry.reason, validity: entry.validity, decisionIds: [...entry.decisionIds],
      citedIn: cited, integrity: run.provenance.integrity.status, rerun: entry.rerun, imported: `ledger/imported/${run.runId}/run.v2.json` });
  }
  const ledger: Ledger = { schemaVersion: 1, classificationSha256: createHash('sha256').update(JSON.stringify(classification)).digest('hex'), entries };
  return { errors, ledger, runs: merged };
}

// Reports, selections and docs that cite a run by its directory name; paths are relative to the repository root.
// Generated catalogs and analyses are excluded: they list runs mechanically and do not select them.
const generated = new Set(['integrated-analysis.json', 'integrated-evidence-catalog.json', 'integrated-source-audit.json', 'integrated-supporting-evidence.json', 'integrated-verification.json']);
export async function findCitations(repositoryRoot: string, runIds: string[]): Promise<Record<string, string[]>> {
  const files: string[] = [];
  for (const [directory, pattern] of [['lab/reports', /\.(md|html|json)$/], ['lab/docs', /\.md$/], ['docs', /\.md$/], ['research', /\.md$/], ['lab', /\.md$/]] as const) {
    for (const entry of await readdir(join(repositoryRoot, directory), { withFileTypes: true }).catch(() => [])) if (entry.isFile() && pattern.test(entry.name) && !generated.has(entry.name)) files.push(join(repositoryRoot, directory, entry.name));
  }
  const citations: Record<string, string[]> = {};
  for (const file of files.sort()) {
    const text = await readFile(file, 'utf8');
    for (const id of runIds) if (text.includes(id)) (citations[id] ??= []).push(relative(repositoryRoot, file));
  }
  return citations;
}
