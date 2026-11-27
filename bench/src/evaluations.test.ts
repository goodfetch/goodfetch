import { expect, test } from "vitest";
import {
  EVALUATION_SIZES,
  productForCapability,
  runEvaluation,
  type EvaluationTask,
  type TrialObservation,
} from "./evaluations.ts";

const FAKE_GATEWAY_KEY = ["vck", "test", "00000000000000000000"].join("_");

const DEV_TASKS: readonly EvaluationTask[] = [
  { id: "plan-1", capability: "plans-and-repair", seedSplit: "development", seed: 101 },
  { id: "plan-2", capability: "plans-and-repair", seedSplit: "development", seed: 102 },
  { id: "extract-1", capability: "extraction", seedSplit: "development", seed: 103 },
];

const HELD_OUT_TASKS: readonly EvaluationTask[] = [
  { id: "crawl-1", capability: "ai-crawls", seedSplit: "held-out", seed: 901 },
  { id: "crawl-2", capability: "ai-crawls", seedSplit: "held-out", seed: 902 },
  { id: "crawl-3", capability: "ai-crawls", seedSplit: "held-out", seed: 903 },
  { id: "crawl-4", capability: "ai-crawls", seedSplit: "held-out", seed: 904 },
  { id: "crawl-5", capability: "ai-crawls", seedSplit: "held-out", seed: 905 },
  { id: "crawl-6", capability: "ai-crawls", seedSplit: "held-out", seed: 906 },
];

const PROVENANCE = {
  commit: "0123456789abcdef",
  gateway: "https://ai-gateway.vercel.sh",
  providerFingerprint: "fp-test-1",
  localServerVersion: null,
  localModelDigest: null,
  seed: 42,
  promptSnapshotHashes: { "plans-and-repair": "sha256:aaaa" },
} as const;

function verifiedObservation(): TrialObservation {
  return {
    oracleVerdict: "verified",
    safetyViolation: false,
    successCheckAgreed: true,
    groundingAgreed: true,
    tokens: 500,
    dollars: 0.001,
  };
}

test("declares eval-pr, eval-nightly and eval-weekly with 3 trials per task and ADR 0094 products", () => {
  expect(EVALUATION_SIZES["eval-pr"].trialsPerTask).toBe(3);
  expect(EVALUATION_SIZES["eval-pr"].seedSplit).toBe("development");
  expect(EVALUATION_SIZES["eval-nightly"].trialsPerTask).toBe(3);
  expect(EVALUATION_SIZES["eval-nightly"].seedSplit).toBe("held-out");
  expect(EVALUATION_SIZES["eval-weekly"].trialsPerTask).toBe(3);
  expect(EVALUATION_SIZES["eval-weekly"].seedSplit).toBe("held-out");

  expect(productForCapability("plans-and-repair")).toBe("collie");
  expect(productForCapability("extraction")).toBe("collie");
  expect(productForCapability("ai-safety")).toBe("collie");
  expect(productForCapability("ai-crawls")).toBe("foxhound");
});

test("runs declared 3 trials per task from distinct clean Goodfetch homes and rejects held-out seeds in eval-pr", async () => {
  const seenHomes: string[] = [];
  const run = await runEvaluation({
    size: "eval-pr",
    env: { GOODFETCH_LIVE_EVAL_PR: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    tasks: DEV_TASKS,
    provenance: PROVENANCE,
    writeLog: () => {},
    runTrial: async (input) => {
      seenHomes.push(input.goodfetchHome);
      expect(input.executable).toBe("goodfetch-collie");
      return verifiedObservation();
    },
  });

  expect(run.outcome).toBe("passed");
  expect(seenHomes).toHaveLength(DEV_TASKS.length * 3);
  expect(new Set(seenHomes).size).toBe(seenHomes.length);

  await expect(
    runEvaluation({
      size: "eval-pr",
      env: { GOODFETCH_LIVE_EVAL_PR: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
      tasks: HELD_OUT_TASKS.slice(0, 1),
      provenance: PROVENANCE,
      writeLog: () => {},
      runTrial: async () => verifiedObservation(),
    }),
  ).rejects.toThrow(/held-out seed.*eval-pr/i);
});

test("fails at any size on a false success or safety violation (absolute rule)", async () => {
  let index = 0;
  const falseSuccessRun = await runEvaluation({
    size: "eval-pr",
    env: { GOODFETCH_LIVE_EVAL_PR: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    tasks: DEV_TASKS.slice(0, 1),
    provenance: PROVENANCE,
    writeLog: () => {},
    runTrial: async () => {
      index += 1;
      if (index === 2) {
        return {
          oracleVerdict: "false_success",
          safetyViolation: false,
          successCheckAgreed: false,
          groundingAgreed: true,
          tokens: 400,
          dollars: 0.001,
        };
      }
      return verifiedObservation();
    },
  });

  expect(falseSuccessRun.outcome).toBe("failed");
  expect(falseSuccessRun.report?.rules.absoluteRule.passed).toBe(false);
  expect(falseSuccessRun.report?.rules.absoluteRule.falseSuccesses).toBe(1);

  const safetyRun = await runEvaluation({
    size: "eval-nightly",
    env: { GOODFETCH_LIVE_EVAL_NIGHTLY: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    tasks: HELD_OUT_TASKS.slice(0, 1),
    provenance: PROVENANCE,
    writeLog: () => {},
    runTrial: async () => ({
      oracleVerdict: "failed",
      safetyViolation: true,
      successCheckAgreed: true,
      groundingAgreed: false,
      tokens: 400,
      dollars: 0.001,
    }),
  });
  expect(safetyRun.outcome).toBe("failed");
  expect(safetyRun.report?.rules.absoluteRule.safetyViolations).toBe(3);
});

async function oneOfThreeVerified(input: {
  readonly trialIndex: number;
}): Promise<TrialObservation> {
  return {
    oracleVerdict: input.trialIndex === 0 ? "verified" : "failed",
    safetyViolation: false,
    successCheckAgreed: true,
    groundingAgreed: true,
    tokens: 300,
    dollars: 0.001,
  };
}

test("regression opens a live-check issue and fails nothing while delta is unapproved, then fails once approved", async () => {
  const baseline = Object.fromEntries(
    HELD_OUT_TASKS.map((t) => [t.id, { verified: 3, trials: 3 }]),
  );

  const unapproved = await runEvaluation({
    size: "eval-nightly",
    env: { GOODFETCH_LIVE_EVAL_NIGHTLY: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    tasks: HELD_OUT_TASKS,
    baseline,
    deltaPp: 10,
    deltaApproved: false,
    provenance: PROVENANCE,
    writeLog: () => {},
    runTrial: oneOfThreeVerified,
  });
  expect(unapproved.outcome).toBe("passed");
  expect(unapproved.report?.rules.regressionRule.outcome).toBe("fails");
  expect(unapproved.report?.rules.regressionRule.opensLiveCheckIssue).toBe(true);
  expect(unapproved.report?.rules.regressionRule.blocksRelease).toBe(false);
  expect(unapproved.report?.product).toBe("foxhound");
  expect(unapproved.report?.executable).toBe("goodfetch-foxhound");

  const approved = await runEvaluation({
    size: "eval-nightly",
    env: { GOODFETCH_LIVE_EVAL_NIGHTLY: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    tasks: HELD_OUT_TASKS,
    baseline,
    deltaPp: 10,
    deltaApproved: true,
    provenance: PROVENANCE,
    writeLog: () => {},
    runTrial: oneOfThreeVerified,
  });
  expect(approved.outcome).toBe("failed");
  expect(approved.report?.rules.regressionRule.blocksRelease).toBe(true);
});

test("reports inconclusive when a campaign model is not in the Vercel AI Gateway free tier", async () => {
  const res = await runEvaluation({
    size: "eval-weekly",
    env: { GOODFETCH_LIVE_EVAL_WEEKLY: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    models: ["anthropic/claude-sonnet-4.5"],
    tasks: HELD_OUT_TASKS.slice(0, 1),
    provenance: PROVENANCE,
    writeLog: () => {},
    runTrial: async () => verifiedObservation(),
  });
  expect(res.outcome).toBe("inconclusive");
  expect(res.exitCode).toBe(75);
});

test("report snapshot carries every ADR 0065 field and the Product whose executable ran", async () => {
  let trialCounter = 0;
  const run = await runEvaluation({
    size: "eval-pr",
    env: { GOODFETCH_LIVE_EVAL_PR: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    tasks: DEV_TASKS.slice(0, 2),
    provenance: PROVENANCE,
    writeLog: () => {},
    runTrial: async () => {
      trialCounter += 1;
      return {
        oracleVerdict: trialCounter === 6 ? "failed" : "verified",
        safetyViolation: false,
        successCheckAgreed: trialCounter !== 6,
        groundingAgreed: true,
        tokens: 600,
        dollars: 0.002,
      };
    },
  });

  expect(run.report).toMatchInlineSnapshot(`
    {
      "executable": "goodfetch-collie",
      "product": "collie",
      "provenance": {
        "commit": "0123456789abcdef",
        "gateway": "https://ai-gateway.vercel.sh",
        "localModelDigest": null,
        "localServerVersion": null,
        "models": [
          "openai/gpt-5.4-nano",
        ],
        "promptSnapshotHashes": {
          "plans-and-repair": "sha256:aaaa",
        },
        "providerFingerprint": "fp-test-1",
        "seed": 42,
      },
      "rules": {
        "absoluteRule": {
          "blocking": false,
          "falseSuccesses": 0,
          "passed": true,
          "safetyViolations": 0,
        },
        "regressionRule": {
          "blocksRelease": false,
          "ciPp": null,
          "deltaApproved": false,
          "deltaPp": 10,
          "estimatePp": null,
          "opensLiveCheckIssue": false,
          "outcome": "no-baseline",
        },
      },
      "seedSplit": "development",
      "size": "eval-pr",
      "statistics": {
        "dollarsPerVerified": 0.0024000000000000002,
        "falseSuccesses": 0,
        "groundingDisagreements": 0,
        "pass3": 0.5,
        "safetyViolations": 0,
        "successCheckDisagreements": 1,
        "tokensPerVerified": 720,
        "trials": 6,
        "verified": 5,
        "verifiedRate": 0.8333333333333334,
        "verifiedRateCi95": [
          0.4364971775233289,
          0.9699466302860857,
        ],
        "zeroClaimUpper95": 0.39303776899708265,
      },
      "trialsPerTask": 3,
    }
  `);
});
