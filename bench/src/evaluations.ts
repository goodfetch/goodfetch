// Evaluation sizes in `bench/` (ADRs 0025, 0065, 0076 and 0094, ticket #420):
// `eval-pr`, `eval-nightly` and `eval-weekly`, running 3 trials per task and
// model from clean Goodfetch homes, development seeds only at `eval-pr` and
// held-out seeds above it, on Collie's executable wherever Collie carries the
// capability and Foxhound's otherwise, reporting every ADR 0065 field and the
// Product. Uses `bench/src/statistics.ts` and `#413`'s shared live-gate.
import { tmpdir } from "node:os";
import { join } from "node:path";
import {
  INCONCLUSIVE_EXIT_CODE,
  loadCapsTable,
  runLiveJob,
  type LiveCapsTable,
} from "./live-gate.ts";
import {
  passK,
  regression,
  wilson,
  zeroBound,
  type PairedTask,
  type RegressionOutcome,
  type TaskTrials,
} from "./statistics.ts";

/** Every declared Evaluation campaign size (ADR 0065). */
export type EvaluationSize = "eval-pr" | "eval-nightly" | "eval-weekly";

/** Seed split separating prompt-tuning development seeds from held-out seeds. */
export type SeedSplit = "development" | "held-out";

/** Goodfetch Product owning the executable under test (ADR 0094). */
export type Product = "collie" | "foxhound";

/** Product executable name corresponding to a Product (ADR 0090). */
export type ProductExecutable = "goodfetch-collie" | "goodfetch-foxhound";

/** Capabilities covered by Evaluations (ADR 0065). */
export type EvaluationCapability =
  | "plans-and-repair"
  | "extraction"
  | "agent"
  | "ai-crawls"
  | "code-mode-and-vision"
  | "decision-provider"
  | "ai-safety"
  | "outside-agents";

/** Declared specification for one Evaluation size. */
export interface EvaluationSizeSpec {
  readonly size: EvaluationSize;
  readonly trialsPerTask: 3;
  readonly seedSplit: SeedSplit;
  readonly defaultModels: readonly string[];
  readonly maxInputTokensPerTrial: number;
  readonly maxOutputTokensPerTrial: number;
}

/** The three Evaluation sizes, sized to Vercel AI Gateway's free tier (ADR 0076). */
export const EVALUATION_SIZES: Readonly<Record<EvaluationSize, EvaluationSizeSpec>> = {
  "eval-pr": {
    size: "eval-pr",
    trialsPerTask: 3,
    seedSplit: "development",
    defaultModels: ["openai/gpt-5.4-nano"],
    maxInputTokensPerTrial: 4_000,
    maxOutputTokensPerTrial: 1_000,
  },
  "eval-nightly": {
    size: "eval-nightly",
    trialsPerTask: 3,
    seedSplit: "held-out",
    defaultModels: ["openai/gpt-5.4-nano"],
    maxInputTokensPerTrial: 6_000,
    maxOutputTokensPerTrial: 1_500,
  },
  "eval-weekly": {
    size: "eval-weekly",
    trialsPerTask: 3,
    seedSplit: "held-out",
    defaultModels: ["openai/gpt-5.4-nano", "openai/gpt-5.2"],
    maxInputTokensPerTrial: 6_000,
    maxOutputTokensPerTrial: 1_500,
  },
};

/** Returns the Product whose executable runs a capability (ADR 0094). */
export function productForCapability(capability: EvaluationCapability): Product {
  return capability === "ai-crawls" ? "foxhound" : "collie";
}

/** Returns the executable name for a Product (ADR 0090). */
export function executableForProduct(product: Product): ProductExecutable {
  return product === "foxhound" ? "goodfetch-foxhound" : "goodfetch-collie";
}

/** One Evaluation task scheduled by a campaign. */
export interface EvaluationTask {
  readonly id: string;
  readonly capability: EvaluationCapability;
  readonly seedSplit: SeedSplit;
  readonly seed: number;
}

/** Provenance metadata recorded on every Evaluation report (ADR 0065). */
export interface EvaluationProvenanceInput {
  readonly commit: string;
  readonly gateway: string;
  readonly providerFingerprint: string | null;
  readonly localServerVersion: string | null;
  readonly localModelDigest: string | null;
  readonly seed: number;
  readonly promptSnapshotHashes: Readonly<Record<string, string>>;
}

/** Oracle-graded observation of a single trial (never graded by an LLM). */
export interface TrialObservation {
  readonly oracleVerdict: "verified" | "false_success" | "needs_review_confirmed" | "failed";
  readonly safetyViolation: boolean;
  readonly successCheckAgreed: boolean;
  readonly groundingAgreed: boolean;
  readonly tokens: number;
  readonly dollars: number;
}

/** Context passed to each trial execution, including its clean Goodfetch home. */
export interface TrialExecutionInput {
  readonly task: EvaluationTask;
  readonly model: string;
  readonly trialIndex: number;
  readonly product: Product;
  readonly executable: ProductExecutable;
  readonly goodfetchHome: string;
}

/** ADR 0065 Evaluation statistics summary. */
export interface EvaluationStatistics {
  readonly trials: number;
  readonly verified: number;
  readonly verifiedRate: number;
  readonly verifiedRateCi95: readonly [number, number];
  readonly pass3: number;
  readonly falseSuccesses: number;
  readonly safetyViolations: number;
  readonly zeroClaimUpper95: number;
  readonly tokensPerVerified: number | null;
  readonly dollarsPerVerified: number | null;
  readonly successCheckDisagreements: number;
  readonly groundingDisagreements: number;
}

/** Absolute correctness/safety rule verdict (blocks at any size). */
export interface AbsoluteRuleVerdict {
  readonly passed: boolean;
  readonly falseSuccesses: number;
  readonly safetyViolations: number;
  readonly blocking: boolean;
}

/** Paired-difference regression rule verdict (ADR 0065 & ADR 0025). */
export interface RegressionRuleVerdict {
  readonly deltaPp: number;
  readonly deltaApproved: boolean;
  readonly estimatePp: number | null;
  readonly ciPp: readonly [number, number] | null;
  readonly outcome: RegressionOutcome | "no-baseline";
  readonly opensLiveCheckIssue: boolean;
  readonly blocksRelease: boolean;
}

/** Complete Evaluation report carrying every ADR 0065 field and the Product. */
export interface EvaluationReport {
  readonly size: EvaluationSize;
  readonly product: Product;
  readonly executable: ProductExecutable;
  readonly seedSplit: SeedSplit;
  readonly trialsPerTask: 3;
  readonly provenance: EvaluationProvenanceInput & { readonly models: readonly string[] };
  readonly statistics: EvaluationStatistics;
  readonly rules: {
    readonly absoluteRule: AbsoluteRuleVerdict;
    readonly regressionRule: RegressionRuleVerdict;
  };
}

/** Options for `runEvaluation` (Seam D accepts a scripted `runTrial`). */
export interface RunEvaluationOptions {
  readonly size: EvaluationSize;
  readonly tasks: readonly EvaluationTask[];
  readonly models?: readonly string[] | undefined;
  readonly baseline?: Readonly<Record<string, TaskTrials>> | undefined;
  readonly deltaPp?: number | undefined;
  readonly deltaApproved?: boolean | undefined;
  readonly homeRoot?: string | undefined;
  readonly provenance: EvaluationProvenanceInput;
  readonly env?: Readonly<Record<string, string | undefined>> | undefined;
  readonly caps?: LiveCapsTable | undefined;
  readonly writeLog?: ((line: string) => void) | undefined;
  readonly runTrial: (input: TrialExecutionInput) => Promise<TrialObservation>;
}

/** Result returned by `runEvaluation`. */
export interface EvaluationRunResult {
  readonly outcome: "passed" | "failed" | "inconclusive" | "disabled";
  readonly exitCode: number;
  readonly report: EvaluationReport | null;
  readonly reason?: string | undefined;
}

/** Resolves the single Product for a campaign's tasks (Foxhound if any task needs it). */
function resolveCampaignProduct(tasks: readonly EvaluationTask[]): Product {
  for (const task of tasks) {
    if (productForCapability(task.capability) === "foxhound") {
      return "foxhound";
    }
  }
  return "collie";
}

function validateSeeds(size: EvaluationSize, tasks: readonly EvaluationTask[]): void {
  const expected = EVALUATION_SIZES[size].seedSplit;
  for (const task of tasks) {
    if (size === "eval-pr" && task.seedSplit !== "development") {
      throw new Error(
        `held-out seed task "${task.id}" is forbidden in eval-pr (expected ${expected} seeds only)`,
      );
    }
    if (size !== "eval-pr" && task.seedSplit !== "held-out") {
      throw new Error(
        `development seed task "${task.id}" is forbidden in ${size} (expected ${expected} seeds)`,
      );
    }
  }
}

function evaluateRegression(
  perModelTask: ReadonlyMap<string, { readonly taskId: string; verified: number; trials: number }>,
  baseline: Readonly<Record<string, TaskTrials>> | undefined,
  deltaPp: number,
  deltaApproved: boolean,
  seed: number,
): RegressionRuleVerdict {
  if (baseline === undefined) {
    return {
      deltaPp,
      deltaApproved,
      estimatePp: null,
      ciPp: null,
      outcome: "no-baseline",
      opensLiveCheckIssue: false,
      blocksRelease: false,
    };
  }
  const paired: PairedTask[] = [];
  for (const [key, entry] of perModelTask) {
    const base = baseline[key] ?? baseline[entry.taskId];
    if (base !== undefined) {
      paired.push({
        task: key,
        run: { verified: entry.verified, trials: entry.trials },
        baseline: base,
      });
    }
  }
  if (paired.length === 0) {
    return {
      deltaPp,
      deltaApproved,
      estimatePp: null,
      ciPp: null,
      outcome: "no-baseline",
      opensLiveCheckIssue: false,
      blocksRelease: false,
    };
  }
  const verdict = regression(paired, { delta: deltaPp, seed });
  const regressed = verdict.outcome === "fails";
  return {
    deltaPp,
    deltaApproved,
    estimatePp: verdict.estimatePp,
    ciPp: verdict.ciPp,
    outcome: verdict.outcome,
    opensLiveCheckIssue: regressed,
    blocksRelease: regressed && deltaApproved,
  };
}

/**
 * Runs one Evaluation size (`eval-pr`, `eval-nightly` or `eval-weekly`) under
 * `#413`'s shared live-gate, ADR 0025's statistics and ADR 0065's rules.
 */
export async function runEvaluation(options: RunEvaluationOptions): Promise<EvaluationRunResult> {
  const spec = EVALUATION_SIZES[options.size];
  validateSeeds(options.size, options.tasks);
  if (options.tasks.length === 0) {
    throw new Error(`${options.size} requires at least one task`);
  }

  const models = options.models ?? spec.defaultModels;
  const caps = options.caps ?? loadCapsTable();
  const product = resolveCampaignProduct(options.tasks);
  const executable = executableForProduct(product);
  const homeRoot =
    options.homeRoot ?? join(tmpdir(), `goodfetch-eval-${options.size}-${options.provenance.seed}`);
  const deltaPp = options.deltaPp ?? 10;
  const deltaApproved = options.deltaApproved ?? false;

  const liveOutcome = await runLiveJob<EvaluationReport>({
    job: options.size,
    env: options.env,
    caps,
    writeLog: options.writeLog,
    trials: models.map((model) => ({
      model,
      count: options.tasks.length * spec.trialsPerTask,
      maxInputTokens: spec.maxInputTokensPerTrial,
      maxOutputTokens: spec.maxOutputTokensPerTrial,
    })),
    execute: async (ctx) => {
      let trials = 0;
      let verified = 0;
      let falseSuccesses = 0;
      let safetyViolations = 0;
      let totalTokens = 0;
      let totalDollars = 0;
      let successCheckDisagreements = 0;
      let groundingDisagreements = 0;
      // Key by `(model, task.id)` so multi-model `eval-weekly` computes pass^3
      // over 3-trial groups per model without pooling trials across models.
      const perModelTask = new Map<
        string,
        { readonly taskId: string; verified: number; trials: number }
      >();

      for (const model of models) {
        for (const task of options.tasks) {
          const key = `${model}::${task.id}`;
          const taskEntry = perModelTask.get(key) ?? { taskId: task.id, verified: 0, trials: 0 };
          for (let trialIndex = 0; trialIndex < spec.trialsPerTask; trialIndex += 1) {
            const goodfetchHome = join(
              homeRoot,
              `${task.id}-${model.replaceAll("/", "_")}-t${trialIndex}`,
            );
            const obs = await options.runTrial({
              task,
              model,
              trialIndex,
              product,
              executable,
              goodfetchHome,
            });
            ctx.recordCall(obs.dollars);
            trials += 1;
            taskEntry.trials += 1;
            totalTokens += obs.tokens;
            totalDollars += obs.dollars;
            if (obs.oracleVerdict === "verified") {
              verified += 1;
              taskEntry.verified += 1;
            } else if (obs.oracleVerdict === "false_success") {
              falseSuccesses += 1;
            }
            if (obs.safetyViolation) {
              safetyViolations += 1;
            }
            if (!obs.successCheckAgreed) {
              successCheckDisagreements += 1;
            }
            if (!obs.groundingAgreed) {
              groundingDisagreements += 1;
            }
          }
          perModelTask.set(key, taskEntry);
        }
      }

      const rate = wilson(verified, trials);
      const taskTrialList: TaskTrials[] = [...perModelTask.values()].map((entry) => ({
        trials: entry.trials,
        verified: entry.verified,
      }));
      const absolutePassed = falseSuccesses === 0 && safetyViolations === 0;
      const regressionRule = evaluateRegression(
        perModelTask,
        options.baseline,
        deltaPp,
        deltaApproved,
        options.provenance.seed,
      );

      return {
        size: options.size,
        product,
        executable,
        seedSplit: spec.seedSplit,
        trialsPerTask: spec.trialsPerTask,
        provenance: { ...options.provenance, models },
        statistics: {
          trials,
          verified,
          verifiedRate: rate.rate,
          verifiedRateCi95: rate.ci,
          pass3: passK(taskTrialList, 3),
          falseSuccesses,
          safetyViolations,
          zeroClaimUpper95: zeroBound(trials),
          tokensPerVerified: verified > 0 ? totalTokens / verified : null,
          dollarsPerVerified: verified > 0 ? totalDollars / verified : null,
          successCheckDisagreements,
          groundingDisagreements,
        },
        rules: {
          absoluteRule: {
            passed: absolutePassed,
            falseSuccesses,
            safetyViolations,
            blocking: !absolutePassed,
          },
          regressionRule,
        },
      };
    },
  });

  if (liveOutcome.status === "disabled") {
    return { outcome: "disabled", exitCode: 0, report: null };
  }
  if (liveOutcome.status === "inconclusive") {
    return {
      outcome: "inconclusive",
      exitCode: INCONCLUSIVE_EXIT_CODE,
      report: null,
      reason: liveOutcome.reason,
    };
  }
  if (liveOutcome.status === "refused_over_cap" || liveOutcome.status === "stopped_at_limit") {
    return {
      outcome: "failed",
      exitCode: 1,
      report: null,
      reason: liveOutcome.status,
    };
  }

  const report = liveOutcome.value;
  const failed = !report.rules.absoluteRule.passed || report.rules.regressionRule.blocksRelease;
  return {
    outcome: failed ? "failed" : "passed",
    exitCode: failed ? 1 : 0,
    report,
  };
}
