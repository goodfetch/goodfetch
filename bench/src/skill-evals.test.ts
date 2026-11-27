import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, test } from "vitest";
import {
  SKILL_EVAL_TRIALS,
  SKILL_TARGETS,
  runSkillEvaluations,
  type SkillTrialResult,
} from "./skill-evals.ts";

const FAKE_GATEWAY_KEY = ["vck", "test", "00000000000000000000"].join("_");

test("promptfoo config and generated skill targets cover both Products with isolation flags", () => {
  const promptfooPath = fileURLToPath(new URL("../promptfoo/skills.yaml", import.meta.url));
  expect(existsSync(promptfooPath)).toBe(true);
  const yaml = readFileSync(promptfooPath, "utf8");
  expect(yaml).toContain("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC");
  expect(yaml).toContain("maxBudgetUsd");
  expect(yaml).toContain("AI_GATEWAY_API_KEY");
  expect(yaml).toContain("skill-used");
  expect(yaml).toContain("goodfetch-collie");
  expect(yaml).toContain("goodfetch-foxhound");

  expect(SKILL_EVAL_TRIALS).toBe(3);
  expect(SKILL_TARGETS.collie.executable).toBe("goodfetch-collie");
  expect(SKILL_TARGETS.foxhound.executable).toBe("goodfetch-foxhound");
});

test("stays disabled until #425 when GOODFETCH_LIVE_SKILLS is unset", async () => {
  let called = 0;
  const result = await runSkillEvaluations({
    env: {},
    products: ["collie"],
    agents: ["claude-code"],
    tierModels: { small: "openai/gpt-5.4-nano", mid: null, large: "openai/gpt-5.2" },
    writeLog: () => {},
    runAgentTrial: async () => {
      called += 1;
      return { oracleVerified: true, skillUsed: true, dollars: 0.001 };
    },
  });
  expect(result.status).toBe("disabled");
  expect(result.exitCode).toBe(0);
  expect(called).toBe(0);
});

test("compares with and without each Product's skills over 3 trials per model, grades by oracle and reports skill-used", async () => {
  const seenExecutables = new Set<string>();
  const seenVariants: string[] = [];
  const isolatedFlagsOk: boolean[] = [];

  const result = await runSkillEvaluations({
    env: { GOODFETCH_LIVE_SKILLS: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    products: ["collie", "foxhound"],
    agents: ["claude-code", "codex"],
    tierModels: { small: "openai/gpt-5.4-nano", mid: null, large: "openai/gpt-5.2" },
    writeLog: () => {},
    runAgentTrial: async (input): Promise<SkillTrialResult> => {
      seenExecutables.add(`${input.product}:${input.executable}`);
      seenVariants.push(
        `${input.product}:${input.agent}:${input.tier}:${input.withSkills ? "with" : "without"}:${input.trialIndex}`,
      );
      isolatedFlagsOk.push(
        input.agent === "claude-code"
          ? input.env["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"] === "1"
          : input.env["CODEX_DISABLE_ANALYTICS"] === "1" &&
              input.env["CODEX_DISABLE_UPDATE_CHECK"] === "1",
      );
      // Collie skills help (3/3 with vs 1/3 without); Foxhound on codex small does not help (1/3 vs 2/3).
      const isUnhelpfulCase =
        input.product === "foxhound" && input.agent === "codex" && input.tier === "small";
      if (isUnhelpfulCase) {
        return {
          oracleVerified: input.withSkills ? input.trialIndex === 0 : input.trialIndex < 2,
          skillUsed: input.withSkills && input.trialIndex === 0,
          dollars: 0.002,
        };
      }
      return {
        oracleVerified: input.withSkills ? true : input.trialIndex === 0,
        skillUsed: input.withSkills,
        dollars: 0.002,
      };
    },
  });

  expect(result.status).toBe("completed");
  expect(result.exitCode).toBe(0);
  expect([...seenExecutables].toSorted()).toEqual([
    "collie:goodfetch-collie",
    "foxhound:goodfetch-foxhound",
  ]);
  // 2 products * 2 agents * 2 free-tier models (small, large; mid is inconclusive) * 2 variants * 3 trials = 48
  expect(seenVariants).toHaveLength(48);
  expect(isolatedFlagsOk.every(Boolean)).toBe(true);
  expect(result.inconclusiveTiers).toEqual(["mid"]);

  const collieSmall = result.comparisons.find(
    (c) => c.product === "collie" && c.agent === "claude-code" && c.tier === "small",
  );
  expect(collieSmall?.withSkillsRate).toBe(1);
  expect(collieSmall?.withoutSkillsRate).toBeCloseTo(1 / 3, 9);
  expect(collieSmall?.skillUsedRate).toBe(1);
  expect(collieSmall?.helped).toBe(true);

  expect(result.unhelpfulSkills).toHaveLength(1);
  expect(result.unhelpfulSkills[0]).toMatchObject({
    product: "foxhound",
    agent: "codex",
    tier: "small",
    helped: false,
  });
});

test("reports overall inconclusive (exit 75) when no target tier has a free-tier model", async () => {
  const result = await runSkillEvaluations({
    env: { GOODFETCH_LIVE_SKILLS: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    products: ["collie"],
    agents: ["claude-code"],
    tierModels: {
      small: "anthropic/claude-sonnet-4.5",
      mid: null,
      large: "anthropic/claude-sonnet-4.5",
    },
    writeLog: () => {},
    runAgentTrial: async () => ({ oracleVerified: true, skillUsed: true, dollars: 0.001 }),
  });

  expect(result.status).toBe("inconclusive");
  expect(result.exitCode).toBe(75);
  expect(result.inconclusiveTiers).toEqual(["small", "mid", "large"]);
});
