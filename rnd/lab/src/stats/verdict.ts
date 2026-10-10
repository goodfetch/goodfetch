// Mechanical verdicts: paired block log-ratios against the decision's smallest effect of interest.
import { readFile, access } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { join } from 'node:path';
import { labRoot } from '../registry.ts';
import { normalQuantile } from './tost.ts';
import { blockBootstrapCI } from './bootstrap.ts';
import { wilcoxonSignedRank, hodgesLehmann } from './rank.ts';
import type { Decision, Registry } from '../core/decisions.ts';

export type Outcome = 'better' | 'worse' | 'equivalent' | 'trivial' | 'non-inferior' | 'inconclusive' | 'blocked-by-contract' | 'insufficient-n' | 'contract-met';
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
type Run = { runId: string; lane: string; host: { classGranted: string }; cells: Cell[]; blocks?: { index: number; noisy: boolean }[] | null };
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
  'admission-no-loss': cells => cells.length > 0 && cells.every(cell => cell.status === 'verified'),
  // A negative variant must be refused without effect; any other outcome is a false success.
  'false-success-count': cells => { const negatives = cells.filter(cell => negativeVariants.includes(String(cell.factors.variant))); return negatives.length > 0 && negatives.every(cell => cell.status === 'verified'); },
  // Identity gates read the variants that test them; each identity cell is judged on the contract of its variant.
  'identity-coherence': cells => ofVariants(cells, ['coherent', 'key-rotation', 'key-revoked']),
  'policy-compliance': cells => ofVariants(cells, policyVariants),
  'session-isolation': cells => ofVariants(cells, ['isolation']),
  'proxy-continuity': cells => ofVariants(cells, ['proxy-sticky', 'proxy-failover', 'proxy-exhausted'])
};
const policyVariants = ['robots-path', 'robots-agent', 'crawl-delay', 'content-signal', 'tdmrep-file', 'tdmrep-header', 'noai-meta', 'paid-402', 'denied-403', 'challenge-page', 'retry-429'];
const negativeVariants = ['never-enabled', 'never-uncovered', 'ambiguous', 'retarget', 'robots-path', 'robots-agent', 'content-signal', 'tdmrep-file', 'tdmrep-header', 'noai-meta', 'paid-402', 'denied-403', 'challenge-page', 'proxy-exhausted'];
const ofVariants = (cells: Cell[], variants: string[]) => { const own = cells.filter(cell => variants.includes(String(cell.factors.variant))); return own.length > 0 && own.every(cell => cell.status === 'verified'); };

export async function evaluateDecision(decision: Decision, { registry, root = labRoot }: { registry: Registry; root?: string }) {
  const analysis = decision.analysis;
  if (decision.kind !== 'technical' || !analysis || !decision.primaryMetric || !decision.rule || !decision.delta) throw new Error(`${decision.id} has no mechanical analysis`);
  const ledgerText = await readFile(join(root, 'ledger/results-ledger.json'), 'utf8');
  const ledger = JSON.parse(ledgerText) as { entries: { runId: string; imported: string }[] };
  const evidence: Run[] = []; const missing: string[] = [];
  for (const item of decision.evidence.filter(entry => entry.kind === 'run')) {
    const entry = ledger.entries.find(row => row.runId === item.ref);
    const path = entry ? join(root, entry.imported) : null;
    if (!path || !(await exists(path))) { missing.push(item.ref); continue; }
    evidence.push(JSON.parse(await readFile(path, 'utf8')) as Run);
  }
  const runs = evidence.filter(run => run.lane === analysis.lane);
  const baselineId = decision.baseline!;
  const optionIds = decision.options.map(option => option.id).filter(id => id !== baselineId && analysis.options[id]);
  // A decision with a single analysed option is a contract decision: its gates are read on that option's cells, lane by lane.
  if (!optionIds.length) {
    if (!analysis.options[baselineId]) throw new Error(`${decision.id}: the baseline is not analysed`);
    const gates = (decision.contractGates ?? []).map(id => {
      const cells = evidence.filter(run => run.lane === (analysis.gateLanes?.[id] ?? analysis.lane)).flatMap(run => run.cells.filter(cell => sameOption(cell.factors, analysis.options[baselineId]!)));
      return { id, ok: gatePredicates[id] ? cells.length > 0 && gatePredicates[id]!(cells) : false, evaluated: Boolean(gatePredicates[id]), cells: cells.length };
    });
    const used = evidence.filter(run => Object.values(analysis.gateLanes ?? {}).concat(analysis.lane).includes(run.lane));
    const observed = used.map(run => run.host.classGranted).sort((a, b) => hostRank.indexOf(a) - hostRank.indexOf(b))[0] ?? null;
    const outcome: Outcome = gates.length && gates.every(gate => gate.ok) ? 'contract-met' : 'blocked-by-contract';
    const primary = { option: baselineId, baseline: baselineId, stratum: analysis.stratum, outcome, n: used.length, margin: null, requiredN: null, estimate: null, gates,
      host: { required: decision.hostClass!.decision, observed, eligible: observed !== null && hostRank.indexOf(observed) >= hostRank.indexOf(decision.hostClass!.decision) } };
    return { schemaVersion: 1, decisionId: decision.id, metricId: decision.primaryMetric.metricId, direction: registry.metrics.find(metric => metric.id === decision.primaryMetric!.metricId)?.direction === 'higher' ? 'higher' : 'lower', rule: decision.rule, delta: decision.delta, ...primary, comparisons: [primary],
      inputs: { runs: used.map(run => run.runId), considered: evidence.map(run => run.runId), missing, ledgerSha256: createHash('sha256').update(ledgerText).digest('hex') } };
  }
  const direction = registry.metrics.find(metric => metric.id === decision.primaryMetric!.metricId)?.direction === 'higher' ? 'higher' : 'lower';
  const seed = parseInt(createHash('sha256').update(decision.id).digest('hex').slice(0, 8), 16);
  const required = decision.hostClass!.decision;
  // Each gate reads the cells of its own lane (by default the measurement lane) for the option and the baseline.
  const gateCells = (gateId: string, optionId: string) => evidence.filter(run => run.lane === (analysis.gateLanes?.[gateId] ?? analysis.lane))
    .flatMap(run => run.cells.filter(cell => sameOption(cell.factors, analysis.options[optionId]!) || sameOption(cell.factors, analysis.options[baselineId]!)));
  // Each declared stratum gets its own comparison; the first one is the decision's primary stratum.
  const strata = analysis.strata?.length ? analysis.strata : [analysis.stratum];
  const compare = (optionId: string, stratum: Record<string, string | number | boolean | null>) => {
    const logRatios: number[] = []; const contributing = new Set<string>();
    for (const run of runs) {
      const ofOption = (id: string) => run.cells.filter(cell => matches(cell.factors, analysis.options[id]!));
      const key = (cell: Cell) => JSON.stringify([cell.factors.block, ...analysis.pairBy.map(factor => cell.factors[factor])]);
      const baselineByKey = new Map(ofOption(baselineId).filter(cell => matches(cell.factors, stratum)).map(cell => [key(cell), cell]));
      // A block whose host check failed is left out of the estimate when the rule excludes noisy blocks.
      const noisy = new Set(decision.rule!.noisyBlockPolicy === 'exclude' ? (run.blocks ?? []).filter(block => block.noisy).map(block => block.index) : []);
      for (const cell of ofOption(optionId).filter(item => matches(item.factors, stratum) && !noisy.has(item.factors.block as number))) {
        const other = baselineByKey.get(key(cell));
        const a = cell.metrics[decision.primaryMetric!.metricId]?.value, b = other?.metrics[decision.primaryMetric!.metricId]?.value;
        if (other && cell.status === 'verified' && other.status === 'verified' && a && b && a > 0 && b > 0) { logRatios.push(Math.log(a / b)); contributing.add(run.runId); }
      }
    }
    // A gate passes only on evidence about the option itself: baseline cells alone never qualify it.
    const gateOk = (id: string) => { const cells = gateCells(id, optionId); return cells.some(cell => sameOption(cell.factors, analysis.options[optionId]!)) && gatePredicates[id]!(cells); };
    const gates = (decision.contractGates ?? []).map(id => ({ id, ok: gatePredicates[id] ? gateOk(id) : false, evaluated: Boolean(gatePredicates[id]) }));
    const result = decide({ logRatios, direction, rule: decision.rule as Rule, delta: decision.delta!.value, gates, seed });
    const used = runs.filter(run => contributing.has(run.runId));
    const observed = (used.length ? used : runs).map(run => run.host.classGranted).sort((a, b) => hostRank.indexOf(a) - hostRank.indexOf(b))[0] ?? null;
    return { option: optionId, baseline: baselineId, stratum, ...result, gates, host: { required, observed, eligible: observed !== null && hostRank.indexOf(observed) >= hostRank.indexOf(required) }, runs: used.map(run => run.runId) };
  };
  const comparisons = optionIds.flatMap(optionId => strata.map(stratum => compare(optionId, stratum)));
  const { runs: primaryRuns, ...primary } = comparisons[0]!;
  return {
    schemaVersion: 1, decisionId: decision.id, metricId: decision.primaryMetric.metricId, direction, rule: decision.rule, delta: decision.delta,
    ...primary, comparisons: comparisons.map(({ runs: _runs, ...comparison }) => comparison),
    inputs: { runs: primaryRuns, considered: evidence.map(run => run.runId), missing, ledgerSha256: createHash('sha256').update(ledgerText).digest('hex') }
  };
}
