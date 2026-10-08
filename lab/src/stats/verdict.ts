// Mechanical verdicts: paired block log-ratios against the decision's smallest effect of interest.
import { readFile, access } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { labRoot } from '../registry.ts';
import { normalQuantile } from './tost.ts';
import { blockBootstrapCI } from './bootstrap.ts';
import { wilcoxonSignedRank, hodgesLehmann } from './rank.ts';
import type { Decision, Registry } from '../core/decisions.ts';

export type Outcome = 'better' | 'worse' | 'equivalent' | 'trivial' | 'non-inferior' | 'inconclusive' | 'blocked-by-contract' | 'insufficient-n';
type Rule = { kind: 'superiority' | 'non-inferiority' | 'equivalence'; alpha: number; power: number };
export type DecideInput = { logRatios: number[]; direction: 'lower' | 'higher'; rule: Rule; delta: number; gates: { id: string; ok: boolean }[]; seed: number; minBlocks?: number };

const mean = (values: number[]) => values.reduce((a, b) => a + b, 0) / values.length;
const sd = (values: number[]) => { const m = mean(values); return Math.sqrt(values.reduce((s, v) => s + (v - m) ** 2, 0) / (values.length - 1)); };

export function decide({ logRatios, direction, rule, delta, gates, seed, minBlocks = 5 }: DecideInput) {
  const n = logRatios.length, margin = Math.log(1 + delta);
  if (gates.some(gate => !gate.ok)) return { outcome: 'blocked-by-contract' as Outcome, n, margin, requiredN: null, estimate: null };
  // Work in improvement space: positive means the option is better than the baseline.
  const improvement = logRatios.map(value => direction === 'lower' ? -value : value);
  const twoSided = rule.kind === 'superiority';
  // Blocks required by the pilot spread for the declared margin, never fewer than the minimum number of process executions.
  const requiredN = n >= 2 ? Math.max(minBlocks, Math.ceil(((normalQuantile(1 - (twoSided ? rule.alpha / 2 : rule.alpha)) + normalQuantile(rule.power)) * sd(improvement) / margin) ** 2)) : null;
  if (requiredN === null || n < requiredN) return { outcome: 'insufficient-n' as Outcome, n, margin, requiredN, estimate: n ? summary(logRatios, seed, rule) : null };
  const estimate = summary(logRatios, seed, rule);
  const [low, high] = direction === 'lower' ? [-estimate.ci[1], -estimate.ci[0]] : estimate.ci;
  let outcome: Outcome = 'inconclusive';
  if (low > margin) outcome = 'better';
  else if (high < -margin) outcome = 'worse';
  else if (rule.kind === 'non-inferiority') outcome = low > -margin ? 'non-inferior' : 'inconclusive';
  else if (low > -margin && high < margin) outcome = low > 0 || high < 0 ? (rule.kind === 'equivalence' ? 'equivalent' : 'trivial') : 'equivalent';
  return { outcome, n, margin, requiredN, estimate };
}

function summary(logRatios: number[], seed: number, rule: Rule) {
  // Superiority uses a two-sided (1 - alpha) interval; equivalence and non-inferiority the TOST (1 - 2 alpha) interval.
  const alpha = rule.kind === 'superiority' ? rule.alpha : 2 * rule.alpha;
  const bootstrap = blockBootstrapCI(logRatios, { resamples: 10_000, seed, alpha });
  return { meanLogRatio: bootstrap.estimate, ratio: Math.exp(bootstrap.estimate), ci: bootstrap.ci, ratioCi: bootstrap.ci.map(Math.exp) as [number, number],
    hodgesLehmannRatio: Math.exp(hodgesLehmann(logRatios)), wilcoxon: wilcoxonSignedRank(logRatios), interval: `${Math.round((1 - alpha) * 100)}% block bootstrap` };
}

type Cell = { factors: Record<string, unknown>; status: string; metrics: Record<string, { value: number | null }> };
type Run = { runId: string; lane: string; host: { classGranted: string }; cells: Cell[] };
const hostRank = ['local-dev', 'local-controlled', 'reference'];
const exists = (path: string) => access(path).then(() => true, () => false);
const matches = (factors: Record<string, unknown>, wanted: Record<string, unknown>) => Object.entries(wanted).every(([key, value]) => value === null ? !(key in factors) : factors[key] === value);
// Gates look at every cell of the compared options, including fault-injection cells that measurement excludes.
const sameOption = (factors: Record<string, unknown>, wanted: Record<string, unknown>) => Object.entries(wanted).every(([key, value]) => value === null || factors[key] === value);

// Gate predicates over the cells of the compared options; gates without a predicate are not evaluated and block.
const gatePredicates: Record<string, (cells: Cell[]) => boolean> = {
  'oracle-exact': cells => cells.length > 0 && cells.every(cell => cell.status === 'verified'),
  'actionability-conformance': cells => cells.length > 0 && cells.every(cell => cell.status === 'verified'),
  'recovery-exact': cells => cells.some(cell => typeof cell.factors.fault === 'string' && cell.factors.fault !== 'none') && cells.every(cell => cell.status === 'verified'),
  'admission-no-loss': cells => cells.length > 0 && cells.every(cell => cell.status === 'verified')
};

export async function evaluateDecision(decision: Decision, { registry, root = labRoot }: { registry: Registry; root?: string }) {
  const analysis = decision.analysis;
  if (decision.kind !== 'technical' || !analysis || !decision.primaryMetric || !decision.rule || !decision.delta) throw new Error(`${decision.id} has no mechanical analysis`);
  const ledgerText = await readFile(join(root, 'ledger/results-ledger.json'), 'utf8');
  const ledger = JSON.parse(ledgerText) as { entries: { runId: string; imported: string }[] };
  const runs: Run[] = []; const missing: string[] = [];
  for (const evidence of decision.evidence.filter(item => item.kind === 'run')) {
    const entry = ledger.entries.find(item => item.runId === evidence.ref);
    const path = entry ? join(root, entry.imported) : null;
    if (!path || !(await exists(path))) { missing.push(evidence.ref); continue; }
    const run = JSON.parse(await readFile(path, 'utf8')) as Run;
    if (run.lane === analysis.lane) runs.push(run);
  }
  const [optionId, baselineId] = [decision.options.find(option => option.id !== decision.baseline && analysis.options[option.id])?.id, decision.baseline!];
  if (!optionId) throw new Error(`${decision.id}: no analysed option besides the baseline`);
  const compared: Cell[] = []; const logRatios: number[] = []; const contributing = new Set<string>();
  for (const run of runs) {
    const ofOption = (id: string) => run.cells.filter(cell => matches(cell.factors, analysis.options[id]!));
    const option = ofOption(optionId), baseline = ofOption(baselineId);
    compared.push(...run.cells.filter(cell => sameOption(cell.factors, analysis.options[optionId]!) || sameOption(cell.factors, analysis.options[baselineId]!)));
    const key = (cell: Cell) => JSON.stringify([cell.factors.block, ...analysis.pairBy.map(factor => cell.factors[factor])]);
    const baselineByKey = new Map(baseline.filter(cell => matches(cell.factors, analysis.stratum)).map(cell => [key(cell), cell]));
    for (const cell of option.filter(item => matches(item.factors, analysis.stratum))) {
      const other = baselineByKey.get(key(cell));
      const a = cell.metrics[decision.primaryMetric.metricId]?.value, b = other?.metrics[decision.primaryMetric.metricId]?.value;
      if (other && cell.status === 'verified' && other.status === 'verified' && a && b && a > 0 && b > 0) { logRatios.push(Math.log(a / b)); contributing.add(run.runId); }
    }
  }
  const gates = (decision.contractGates ?? []).map(id => ({ id, ok: gatePredicates[id] ? gatePredicates[id]!(compared) : false, evaluated: Boolean(gatePredicates[id]) }));
  const direction = registry.metrics.find(metric => metric.id === decision.primaryMetric!.metricId)?.direction === 'higher' ? 'higher' : 'lower';
  const seed = parseInt(createHash('sha256').update(decision.id).digest('hex').slice(0, 8), 16);
  const result = decide({ logRatios, direction, rule: decision.rule as Rule, delta: decision.delta.value, gates, seed });
  const used = runs.filter(run => contributing.has(run.runId));
  const observed = (used.length ? used : runs).map(run => run.host.classGranted).sort((a, b) => hostRank.indexOf(a) - hostRank.indexOf(b))[0] ?? null;
  const required = decision.hostClass!.decision;
  return {
    schemaVersion: 1, decisionId: decision.id, metricId: decision.primaryMetric.metricId, direction, option: optionId, baseline: baselineId, stratum: analysis.stratum,
    rule: decision.rule, delta: decision.delta, ...result, gates,
    host: { required, observed, eligible: observed !== null && hostRank.indexOf(observed) >= hostRank.indexOf(required) },
    inputs: { runs: used.map(run => run.runId), considered: runs.map(run => run.runId), missing, ledgerSha256: createHash('sha256').update(ledgerText).digest('hex') }
  };
}
