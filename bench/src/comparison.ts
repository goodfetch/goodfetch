// A decision's verdicts on the runs it cites, carried over from the lab's
// `evaluateDecision`: each option against the baseline on the primary
// metric, paired by block and task (ADR 0025's pairing unit), one comparison
// for each declared stratum of tasks. A pair counts when both cells passed
// their gate and measured the metric above zero. The contract gates read the
// option's own cells beside the baseline's, so an option with no cells fails
// them; the host is eligible when the lowest class among the runs used is
// the decision's or above.
import { createHash } from "node:crypto";
import type { Cell } from "./cell.ts";
import { hostClasses, type HostClass } from "./host.ts";
import { metricCatalog, type MetricId } from "./metrics.ts";
import type { ContractGate, Decision } from "./registry.ts";
import { decide, type Outcome, type Verdict } from "./verdict.ts";

/** The part of a cell a comparison reads. */
export interface ComparedCell extends Pick<Cell, "adapter" | "task" | "block" | "gate" | "counts"> {
  readonly metrics: Readonly<Partial<Record<MetricId, { readonly value: number | null }>>>;
}

/** The part of a run record a comparison reads. */
export interface ComparedRun {
  readonly run_id: string;
  readonly host: { readonly class_granted: HostClass };
  readonly blocks: readonly { readonly block: number; readonly noisy: boolean }[];
  readonly cells: readonly ComparedCell[];
}

/** One option against the baseline in one stratum. */
export interface Comparison extends Verdict {
  readonly option: string;
  readonly baseline: string;
  /** The stratum's ID, or `null` for every task. */
  readonly stratum: string | null;
  readonly gates: readonly { readonly id: ContractGate; readonly ok: boolean }[];
  readonly host: {
    readonly required: HostClass;
    readonly observed: HostClass | null;
    readonly eligible: boolean;
  };
  /** The runs whose pairs it read. */
  readonly runs: readonly string[];
}

/** A decision's comparisons, the first its primary one, and the runs they read. */
export interface DecisionVerdict {
  readonly decision: string;
  readonly metric: MetricId;
  readonly direction: "lower" | "higher";
  readonly comparisons: readonly Comparison[];
  /** The cited runs found, and those cited but not given. */
  readonly inputs: { readonly runs: readonly string[]; readonly missing: readonly string[] };
}

/** The outcomes a noisy block turns inconclusive when the rule says so. */
const decisive: ReadonlySet<Outcome> = new Set([
  "better",
  "worse",
  "equivalent",
  "trivial",
  "non-inferior",
]);

const gates: Readonly<Record<ContractGate, (cells: readonly ComparedCell[]) => boolean>> = {
  // Every cell passed its gate, and the oracle judged none of its answers wrong.
  oracle_exact: (cells) =>
    cells.every(
      (cell) =>
        cell.gate === "passed" &&
        cell.counts.failed + cell.counts.false_success + cell.counts.incomplete === 0,
    ),
  false_success_count: (cells) => cells.every((cell) => cell.counts.false_success === 0),
};

function lowestClass(runs: readonly ComparedRun[]): HostClass | null {
  const ranks = runs.map((run) => hostClasses.indexOf(run.host.class_granted));
  return ranks.length === 0 ? null : (hostClasses[Math.min(...ranks)] ?? null);
}

/** The verdicts of `decision` on those of `runs` it cites as evidence. */
export function evaluateDecision(
  decision: Decision,
  runs: readonly ComparedRun[],
): DecisionVerdict {
  const metric = decision.primary_metric;
  const { direction } = metricCatalog[metric];
  const evidence = runs.filter((run) => decision.evidence.includes(run.run_id));
  const found = new Set(evidence.map((run) => run.run_id));
  const adapterOf = new Map(decision.options.map((option) => [option.id, option.adapter]));
  const baseline = adapterOf.get(decision.baseline);
  if (baseline === undefined) {
    throw new Error(`invariant: ${decision.id}'s baseline is one of its options`);
  }
  const seed = Number.parseInt(
    createHash("sha256").update(decision.id).digest("hex").slice(0, 8),
    16,
  );
  const required = decision.host_class.decision;
  const strata = decision.strata ?? [{ id: null, tasks: null }];

  const compare = (
    option: string,
    adapter: string,
    stratum: { readonly id: string | null; readonly tasks: readonly string[] | null },
  ): Comparison => {
    const inStratum = (cell: ComparedCell) =>
      stratum.tasks === null || stratum.tasks.includes(cell.task);
    const logRatios: number[] = [];
    const contributing = new Set<string>();
    let noisyPairs = 0;
    for (const run of evidence) {
      const key = (cell: ComparedCell) => `${cell.block}\u0000${cell.task}`;
      const baselineCells = new Map(
        run.cells
          .filter((cell) => cell.adapter === baseline && inStratum(cell))
          .map((cell) => [key(cell), cell]),
      );
      const noisy = new Set(run.blocks.filter((block) => block.noisy).map((block) => block.block));
      for (const cell of run.cells.filter((one) => one.adapter === adapter && inStratum(one))) {
        const other = baselineCells.get(key(cell));
        if (noisy.has(cell.block) && decision.rule.noisy_blocks === "exclude") {
          continue;
        }
        const a = cell.metrics[metric]?.value;
        const b = other?.metrics[metric]?.value;
        if (
          other?.gate === "passed" &&
          cell.gate === "passed" &&
          typeof a === "number" &&
          typeof b === "number" &&
          a > 0 &&
          b > 0
        ) {
          logRatios.push(Math.log(a / b));
          contributing.add(run.run_id);
          noisyPairs += noisy.has(cell.block) ? 1 : 0;
        }
      }
    }
    const gateCells = evidence.flatMap((run) =>
      run.cells.filter((cell) => cell.adapter === adapter || cell.adapter === baseline),
    );
    const ownCells = gateCells.some((cell) => cell.adapter === adapter);
    const judged = decision.contract_gates.map((id) => ({
      id,
      ok: ownCells && gates[id](gateCells),
    }));
    const verdict = decide({
      logRatios,
      direction,
      rule: decision.rule,
      delta: decision.delta,
      gates: judged,
      seed,
    });
    const outcome =
      noisyPairs > 0 && decisive.has(verdict.outcome) ? "inconclusive" : verdict.outcome;
    const used = evidence.filter((run) => contributing.has(run.run_id));
    const observed = lowestClass(used.length > 0 ? used : evidence);
    return {
      ...verdict,
      outcome,
      option,
      baseline: decision.baseline,
      stratum: stratum.id,
      gates: judged,
      host: {
        required,
        observed,
        eligible:
          observed !== null && hostClasses.indexOf(observed) >= hostClasses.indexOf(required),
      },
      runs: used.map((run) => run.run_id),
    };
  };

  return {
    decision: decision.id,
    metric,
    direction,
    comparisons: decision.options
      .filter((option) => option.id !== decision.baseline)
      .flatMap((option) => strata.map((stratum) => compare(option.id, option.adapter, stratum))),
    inputs: {
      runs: [...found],
      missing: decision.evidence.filter((id) => !found.has(id)),
    },
  };
}
