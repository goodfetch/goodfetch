import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { join } from 'node:path';
import { labRoot } from '../registry.ts';
import { validateDocument } from './schema.ts';

export type Metric = { id: string; kind: 'performance' | 'resource' | 'quality' | 'cost' | 'contract'; unit: string; direction: string; description: string; platforms: { darwin: string | null; linux: string | null } };
export type Decision = {
  id: string; title: string; question: string; kind: 'technical' | 'product'; status: string;
  options: { id: string; description: string; adapters?: string[] }[]; baseline?: string;
  primaryMetric?: { metricId: string; blockStatistic: string }; secondaryMetrics?: string[]; contractGates?: string[];
  delta?: { kind: string; value: number }; rule?: { kind: string; alpha: number; power: number; noisyBlockPolicy: string };
  hostClass?: { decision: string; publication: string }; networkProfiles?: unknown[]; campaigns?: string[];
  evidence: { kind: 'run' | 'research-doc'; ref: string; note?: string }[]; verdictRef?: string | null;
  analysis?: { lane: string; options: Record<string, Record<string, string | number | boolean | null>>; pairBy: string[]; stratum: Record<string, string | number | boolean | null> }; adr?: string | null; dependsOn: string[]; notes: string;
};
export type Claim = { id: string; statement: string; decisionId: string; metricId: string; comparator: { option: string; against: string[] }; direction: string; publicationHostClass: string; status: string; evidence: string[]; notes: string };
export type Registry = { root: string; decisions: Decision[]; claims: Claim[]; metrics: Metric[]; ledgerRunIds: string[] | null };

const readJson = (path: string) => JSON.parse(readFileSync(path, 'utf8')) as unknown;
const readDirectory = (directory: string) => existsSync(directory) ? readdirSync(directory).filter(name => name.endsWith('.json')).sort().map(name => readJson(join(directory, name))) : [];

export function loadRegistry(root = labRoot): Registry {
  const catalog = readJson(join(root, 'metrics/catalog.json')) as { metrics: Metric[] };
  const ledgerPath = join(root, 'ledger/results-ledger.json');
  const ledger = existsSync(ledgerPath) ? readJson(ledgerPath) as { entries: { runId: string }[] } : null;
  return { root, decisions: readDirectory(join(root, 'decisions')) as Decision[], claims: readDirectory(join(root, 'claims')) as Claim[], metrics: catalog.metrics, ledgerRunIds: ledger ? ledger.entries.map(entry => entry.runId) : null };
}

// Measured figures belong to generated verdicts; editorial text must not restate them.
export const measuredFigure = /\d+(?:[.,]\d+)?\s*(?:%|ms\b|s\b|µs\b|[KMG]i?B\b|[KMG]o\b|B\b|jobs?\/s|req(?:uests)?\/s|rows?\/s|tasks?\/s|×|x\b|fois\b)/i;

// Returns the first measured figure found in editorial text, or null.
export function lintEditorial(text: string): string | null { return text.match(new RegExp(measuredFigure.source, 'i'))?.[0] ?? null; }

export function checkRegistry(registry: Registry): string[] {
  const errors: string[] = [];
  const metrics = new Map(registry.metrics.map(metric => [metric.id, metric]));
  const decisions = new Map(registry.decisions.map(decision => [decision.id, decision]));
  for (const problem of validateDocument('metric-catalog', { schemaVersion: 1, metrics: registry.metrics })) errors.push(`metrics/catalog.json: ${problem}`);
  for (const id of duplicates(registry.metrics.map(metric => metric.id))) errors.push(`metric ${id}: duplicate id`);
  for (const id of duplicates(registry.decisions.map(decision => decision.id))) errors.push(`${id}: duplicate decision id`);
  for (const decision of registry.decisions) {
    const at = (message: string) => errors.push(`${decision.id}: ${message}`);
    for (const problem of validateDocument('decision', decision)) at(problem);
    const options = new Set(decision.options?.map(option => option.id));
    if (decision.baseline !== undefined && !options.has(decision.baseline)) at(`baseline ${decision.baseline} is not one of its options`);
    for (const metricId of [decision.primaryMetric?.metricId, ...(decision.secondaryMetrics ?? [])]) if (metricId && !metrics.has(metricId)) at(`unknown metric ${metricId}`);
    for (const gate of decision.contractGates ?? []) if (metrics.get(gate)?.kind !== 'contract') at(`contract gate ${gate} is not a contract metric of the catalog`);
    for (const dependency of decision.dependsOn ?? []) if (!decisions.has(dependency)) at(`depends on unknown decision ${dependency}`);
    if (measuredFigure.test(decision.notes ?? '')) at('notes contain a measured figure; figures must come from verdicts');
    for (const evidence of decision.evidence ?? []) {
      if (evidence.kind === 'run' && registry.ledgerRunIds && !registry.ledgerRunIds.includes(evidence.ref)) at(`evidence run ${evidence.ref} is not in the results ledger`);
      if (evidence.kind === 'research-doc' && !existsSync(join(registry.root, '..', evidence.ref))) at(`evidence document ${evidence.ref} does not exist`);
    }
  }
  for (const id of duplicates(registry.claims.map(claim => claim.id))) errors.push(`${id}: duplicate claim id`);
  for (const claim of registry.claims) {
    const at = (message: string) => errors.push(`${claim.id}: ${message}`);
    for (const problem of validateDocument('claim', claim)) at(problem);
    const decision = decisions.get(claim.decisionId);
    if (!decision) { at(`refers to unknown decision ${claim.decisionId}`); continue; }
    if (!metrics.has(claim.metricId)) at(`unknown metric ${claim.metricId}`);
    const options = new Set(decision.options.map(option => option.id));
    for (const option of [claim.comparator.option, ...claim.comparator.against]) if (!options.has(option)) at(`option ${option} is not an option of ${decision.id}`);
    if (measuredFigure.test(claim.statement) || measuredFigure.test(claim.notes)) at('statement or notes contain a measured figure');
  }
  return errors;
}

function duplicates(ids: string[]) { return ids.filter((id, index) => ids.indexOf(id) !== index); }
