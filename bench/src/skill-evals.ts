// Skill Evaluations campaign (ADRs 0065, 0076, 0090 and 0094; ticket #423):
// drives promptfoo (`bench/promptfoo/skills.yaml`) / coding agents (Claude
// Code via the Claude Agent SDK with `maxBudgetUsd`, and Codex via Vercel AI
// Gateway) against each Product's own executable (`goodfetch-collie` for
// Collie's skills, `goodfetch-foxhound` for Foxhound's skills) and the local
// Proving ground, comparing with vs without skills over 3 trials per target
// model tier (`small`, `mid`, `large`), graded strictly by the oracle with
// `skill-used` reported as a triggering signal. Runs under `#413`'s shared
// live-gate and stays disabled until `#425`.
import { execFile } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import type { Product, ProductExecutable } from "./evaluations.ts";
import {
  INCONCLUSIVE_EXIT_CODE,
  claudeCodeLiveEnv,
  codexLiveEnv,
  loadCapsTable,
  runLiveJob,
  type LiveCapsTable,
} from "./live-gate.ts";

const execFileAsync = promisify(execFile);

/** Number of trials run per (product, agent, model tier, with/without skills). */
export const SKILL_EVAL_TRIALS = 3;

/** Model tiers targeted by Goodfetch's agent skills (ADR 0065). */
export type SkillModelTier = "small" | "mid" | "large";

const SKILL_MODEL_TIERS: readonly SkillModelTier[] = ["small", "mid", "large"];

/** Coding agents evaluated on Goodfetch's skills. */
export type SkillAgent = "claude-code" | "codex";

/** Product-to-executable and generated skills directory mapping (ADRs 0090 & 0094). */
export interface ProductSkillTarget {
  readonly product: Product;
  readonly executable: ProductExecutable;
  readonly skillsDir: string;
}

/** Each Product's skills run against that Product's own executable. */
export const SKILL_TARGETS: Readonly<Record<Product, ProductSkillTarget>> = {
  collie: {
    product: "collie",
    executable: "goodfetch-collie",
    skillsDir: "crates/goodfetch-collie/generated/skills",
  },
  foxhound: {
    product: "foxhound",
    executable: "goodfetch-foxhound",
    skillsDir: "crates/foxhound/goodfetch-foxhound/generated/skills",
  },
};

/** Input passed to each scripted or live coding-agent trial. */
export interface SkillTrialInput {
  readonly product: Product;
  readonly executable: ProductExecutable;
  readonly skillsDir: string | null;
  readonly agent: SkillAgent;
  readonly tier: SkillModelTier;
  readonly model: string;
  readonly withSkills: boolean;
  readonly trialIndex: number;
  readonly env: Readonly<Record<string, string>>;
}

/** Oracle-graded outcome of one coding-agent trial plus promptfoo's `skill-used` signal. */
export interface SkillTrialResult {
  readonly oracleVerified: boolean;
  readonly skillUsed: boolean;
  readonly dollars: number;
}

/** Paired with-skills vs without-skills comparison for one (product, agent, tier). */
export interface SkillComparison {
  readonly product: Product;
  readonly executable: ProductExecutable;
  readonly agent: SkillAgent;
  readonly tier: SkillModelTier;
  readonly model: string;
  readonly trialsPerVariant: number;
  readonly withSkillsVerified: number;
  readonly withoutSkillsVerified: number;
  readonly withSkillsRate: number;
  readonly withoutSkillsRate: number;
  readonly liftPp: number;
  readonly skillUsedCount: number;
  readonly skillUsedRate: number;
  /** True iff with-skills oracle rate strictly improves over baseline (ADR 0065). */
  readonly helped: boolean;
}

/** Options for `runSkillEvaluations` (Seam D accepts scripted `runAgentTrial`). */
export interface RunSkillEvaluationsOptions {
  readonly products: readonly Product[];
  readonly agents: readonly SkillAgent[];
  readonly tierModels: Readonly<Record<SkillModelTier, string | null>>;
  readonly maxBudgetUsdPerTrial?: number | undefined;
  readonly env?: Readonly<Record<string, string | undefined>> | undefined;
  readonly caps?: LiveCapsTable | undefined;
  readonly writeLog?: ((line: string) => void) | undefined;
  readonly runAgentTrial: (input: SkillTrialInput) => Promise<SkillTrialResult>;
}

/** Overall outcome of the Skill Evaluations campaign. */
export interface SkillEvaluationsOutcome {
  readonly status: "disabled" | "completed" | "inconclusive" | "failed";
  readonly exitCode: number;
  readonly comparisons: readonly SkillComparison[];
  readonly unhelpfulSkills: readonly SkillComparison[];
  readonly inconclusiveTiers: readonly SkillModelTier[];
  readonly reason?: string | undefined;
}

/** Creates a live trial runner invoking `promptfoo eval` against `bench/promptfoo/skills.yaml`. */
export function createPromptfooTrialRunner(
  configPath: string = fileURLToPath(new URL("../promptfoo/skills.yaml", import.meta.url)),
): (input: SkillTrialInput) => Promise<SkillTrialResult> {
  return async (input) => {
    const dir = mkdtempSync(join(tmpdir(), "gf-promptfoo-"));
    const outPath = join(dir, "result.json");
    const providerLabel = `${input.agent}-${input.withSkills ? "with-skills" : "without-skills"}`;
    try {
      await execFileAsync(
        "promptfoo",
        [
          "eval",
          "--config",
          configPath,
          "--filter-providers",
          providerLabel,
          "--filter-pattern",
          input.product,
          "--var",
          `model=${input.model}`,
          "--output",
          outPath,
          "--no-cache",
        ],
        {
          env: {
            PATH: process.env["PATH"],
            ...input.env,
            GOODFETCH_EVAL_PRODUCT: input.product,
            GOODFETCH_EVAL_EXECUTABLE: input.executable,
          },
        },
      );
      const raw: unknown = JSON.parse(readFileSync(outPath, "utf8"));
      const text = JSON.stringify(raw);
      return {
        oracleVerified: text.includes('"success":true'),
        skillUsed: text.includes("skill-used") && !text.includes('"skill-used":false'),
        dollars: 0.01,
      };
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  };
}

function partitionTiers(
  tierModels: Readonly<Record<SkillModelTier, string | null>>,
  caps: LiveCapsTable,
): {
  readonly runnable: readonly { readonly tier: SkillModelTier; readonly model: string }[];
  readonly inconclusive: readonly SkillModelTier[];
} {
  const runnable: { readonly tier: SkillModelTier; readonly model: string }[] = [];
  const inconclusive: SkillModelTier[] = [];
  for (const tier of SKILL_MODEL_TIERS) {
    const model = tierModels[tier];
    if (model === null || caps.models[model]?.freeTier !== true) {
      inconclusive.push(tier);
    } else {
      runnable.push({ tier, model });
    }
  }
  return { runnable, inconclusive };
}

/**
 * Runs Skill Evaluations (`skills` campaign) under `#413`'s shared live-gate:
 * stays disabled until `#425`, reports inconclusive for tiers lacking a
 * Vercel AI Gateway free-tier model, grades solely by oracle, and reports
 * `skill-used` and every skill that fails to improve over baseline.
 */
export async function runSkillEvaluations(
  options: RunSkillEvaluationsOptions,
): Promise<SkillEvaluationsOutcome> {
  const caps = options.caps ?? loadCapsTable();
  const maxBudgetUsd = options.maxBudgetUsdPerTrial ?? 2;
  const { runnable, inconclusive } = partitionTiers(options.tierModels, caps);

  // If disabled by repo var, return cleanly before examining model tiers.
  const firstModel = runnable[0]?.model ?? "openai/gpt-5.4-nano";
  const plannedTrials =
    runnable.length === 0
      ? [{ model: firstModel, count: 0, maxInputTokens: 0, maxOutputTokens: 0 }]
      : runnable.map(({ model }) => ({
          model,
          count: options.products.length * options.agents.length * 2 * SKILL_EVAL_TRIALS,
          maxInputTokens: 6_000,
          maxOutputTokens: 2_000,
        }));

  const liveOutcome = await runLiveJob<{
    readonly comparisons: readonly SkillComparison[];
    readonly inconclusiveAll: boolean;
  }>({
    job: "skills",
    env: options.env,
    caps,
    writeLog: options.writeLog,
    trials: plannedTrials,
    execute: async (ctx) => {
      if (runnable.length === 0) {
        return { comparisons: [], inconclusiveAll: true };
      }

      const comparisons: SkillComparison[] = [];
      for (const product of options.products) {
        const target = SKILL_TARGETS[product];
        for (const agent of options.agents) {
          const agentEnv =
            agent === "claude-code"
              ? claudeCodeLiveEnv({
                  apiKey: ctx.apiKey,
                  gatewayBaseUrl: ctx.gateway,
                  maxBudgetUsd,
                })
              : codexLiveEnv({
                  apiKey: ctx.apiKey,
                  gatewayBaseUrl: ctx.gateway,
                });

          for (const { tier, model } of runnable) {
            let withVerified = 0;
            let withoutVerified = 0;
            let skillUsedCount = 0;

            for (const withSkills of [false, true]) {
              for (let trialIndex = 0; trialIndex < SKILL_EVAL_TRIALS; trialIndex += 1) {
                const res = await options.runAgentTrial({
                  product: target.product,
                  executable: target.executable,
                  skillsDir: withSkills ? target.skillsDir : null,
                  agent,
                  tier,
                  model,
                  withSkills,
                  trialIndex,
                  env: agentEnv,
                });
                ctx.recordCall(res.dollars);
                if (withSkills) {
                  if (res.oracleVerified) {
                    withVerified += 1;
                  }
                  if (res.skillUsed) {
                    skillUsedCount += 1;
                  }
                } else if (res.oracleVerified) {
                  withoutVerified += 1;
                }
              }
            }

            const withSkillsRate = withVerified / SKILL_EVAL_TRIALS;
            const withoutSkillsRate = withoutVerified / SKILL_EVAL_TRIALS;
            comparisons.push({
              product: target.product,
              executable: target.executable,
              agent,
              tier,
              model,
              trialsPerVariant: SKILL_EVAL_TRIALS,
              withSkillsVerified: withVerified,
              withoutSkillsVerified: withoutVerified,
              withSkillsRate,
              withoutSkillsRate,
              liftPp: (withSkillsRate - withoutSkillsRate) * 100,
              skillUsedCount,
              skillUsedRate: skillUsedCount / SKILL_EVAL_TRIALS,
              helped: withSkillsRate > withoutSkillsRate,
            });
          }
        }
      }
      return { comparisons, inconclusiveAll: false };
    },
  });

  if (liveOutcome.status === "disabled") {
    return {
      status: "disabled",
      exitCode: 0,
      comparisons: [],
      unhelpfulSkills: [],
      inconclusiveTiers: inconclusive,
    };
  }
  if (liveOutcome.status === "inconclusive") {
    return {
      status: "inconclusive",
      exitCode: INCONCLUSIVE_EXIT_CODE,
      comparisons: [],
      unhelpfulSkills: [],
      inconclusiveTiers: inconclusive,
      reason: liveOutcome.reason,
    };
  }
  if (liveOutcome.status === "refused_over_cap" || liveOutcome.status === "stopped_at_limit") {
    return {
      status: "failed",
      exitCode: 1,
      comparisons: [],
      unhelpfulSkills: [],
      inconclusiveTiers: inconclusive,
      reason: liveOutcome.status,
    };
  }
  if (liveOutcome.value.inconclusiveAll) {
    return {
      status: "inconclusive",
      exitCode: INCONCLUSIVE_EXIT_CODE,
      comparisons: [],
      unhelpfulSkills: [],
      inconclusiveTiers: inconclusive,
      reason: "no target model tier has a Vercel AI Gateway free-tier model",
    };
  }

  const comparisons = liveOutcome.value.comparisons;
  const unhelpfulSkills = comparisons.filter((c) => !c.helped);
  return {
    status: "completed",
    exitCode: 0,
    comparisons,
    unhelpfulSkills,
    inconclusiveTiers: inconclusive,
  };
}
