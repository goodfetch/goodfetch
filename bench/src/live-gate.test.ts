import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { expect, test } from "vitest";
import {
  CurrencyLimitTracker,
  INCONCLUSIVE_EXIT_CODE,
  classifyGatewayResponse,
  claudeCodeLiveEnv,
  codexIsolatedConfigToml,
  codexLiveEnv,
  estimateWorstCaseUsd,
  loadCapsTable,
  runLiveJob,
  type LiveJobId,
} from "./live-gate.ts";

// One visibly fake constant built in code so no real-looking token literal lives in tests.
const FAKE_GATEWAY_KEY = ["vck", "test", "00000000000000000000"].join("_");

test("committed cap table matches ADR 0065 per-run caps and free-tier models", () => {
  const caps = loadCapsTable();
  expect(caps.gateway).toBe("https://ai-gateway.vercel.sh");
  expect(caps.apiKeyEnv).toBe("AI_GATEWAY_API_KEY");
  const expectedCaps: readonly (readonly [LiveJobId, number])[] = [
    ["provider-check", 0.5],
    ["recorder", 2],
    ["eval-pr", 2],
    ["eval-nightly", 4],
    ["eval-weekly", 35],
    ["skills", 35],
  ];
  for (const [job, capUsd] of expectedCaps) {
    expect(caps.jobs[job].capUsd).toBe(capUsd);
  }
  expect(caps.models["openai/gpt-5.4-nano"]?.freeTier).toBe(true);
  expect(caps.models["openai/gpt-5.2"]?.freeTier).toBe(true);
  expect(caps.models["anthropic/claude-sonnet-4.5"]?.freeTier).toBe(false);
});

test("exits at step 1 without any call when the repository variable is unset", async () => {
  let calls = 0;
  const logs: string[] = [];
  const outcome = await runLiveJob({
    job: "provider-check",
    env: {},
    trials: [
      { model: "openai/gpt-5.4-nano", count: 1, maxInputTokens: 1_000, maxOutputTokens: 500 },
    ],
    writeLog: (line) => logs.push(line),
    execute: async () => {
      calls += 1;
      return "should not run";
    },
  });

  expect(outcome.status).toBe("disabled");
  expect(outcome.exitCode).toBe(0);
  expect(outcome.callsMade).toBe(0);
  expect(calls).toBe(0);
  expect(logs.join("\n")).toContain("GOODFETCH_LIVE_PROVIDER_CHECK");
});

test("prints plan and worst case before the first call and refuses when worst case exceeds cap", async () => {
  const events: string[] = [];
  const overCap = await runLiveJob({
    job: "provider-check",
    env: { GOODFETCH_LIVE_PROVIDER_CHECK: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    trials: [
      { model: "openai/gpt-5.2", count: 100, maxInputTokens: 2_000, maxOutputTokens: 1_000 },
    ],
    writeLog: (line) => events.push(`log:${line}`),
    execute: async () => {
      events.push("call");
      return "should not run";
    },
  });

  expect(overCap.status).toBe("refused_over_cap");
  expect(overCap.exitCode).toBe(1);
  expect(overCap.callsMade).toBe(0);
  expect(events.some((entry) => entry === "call")).toBe(false);
  expect(events.join("\n")).toContain("worst case");
  expect(events.join("\n")).toContain("openai/gpt-5.2");

  const okEvents: string[] = [];
  const underCap = await runLiveJob({
    job: "provider-check",
    env: { GOODFETCH_LIVE_PROVIDER_CHECK: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    trials: [
      { model: "openai/gpt-5.4-nano", count: 3, maxInputTokens: 1_000, maxOutputTokens: 500 },
    ],
    writeLog: (line) => okEvents.push(`log:${line}`),
    execute: async (ctx) => {
      okEvents.push("call");
      ctx.recordCall(0.001);
      return "ok";
    },
  });

  expect(underCap.status).toBe("completed");
  expect(underCap.exitCode).toBe(0);
  expect(okEvents[0]).toMatch(/^log:/);
  expect(okEvents.indexOf("call")).toBeGreaterThan(0);
});

test("stops a run when it reaches its currency Limit", async () => {
  let completedCalls = 0;
  const outcome = await runLiveJob({
    job: "provider-check",
    env: { GOODFETCH_LIVE_PROVIDER_CHECK: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    trials: [
      { model: "openai/gpt-5.4-nano", count: 5, maxInputTokens: 1_000, maxOutputTokens: 500 },
    ],
    writeLog: () => {},
    execute: async (ctx) => {
      for (let i = 0; i < 5; i += 1) {
        if (!ctx.canAfford(0.2)) {
          return ctx.stopAtLimit();
        }
        ctx.recordCall(0.2);
        completedCalls += 1;
      }
      return "done";
    },
  });

  expect(outcome.status).toBe("stopped_at_limit");
  expect(outcome.exitCode).toBe(1);
  expect(completedCalls).toBe(2);
  expect(outcome.spentUsd).toBeCloseTo(0.4, 9);
});

test("reports a 429 or exhausted free credit as inconclusive (exit 75)", async () => {
  expect(classifyGatewayResponse({ status: 429, retryAfterSeconds: 30 })).toEqual({
    kind: "inconclusive",
    reason: "rate_limit",
    status: 429,
    retryAfterSeconds: 30,
  });
  expect(
    classifyGatewayResponse({
      status: 402,
      errorCode: "quota_for_entity_exceeded",
      body: "Insufficient credits on free tier",
    }),
  ).toEqual({
    kind: "inconclusive",
    reason: "exhausted_credit",
    status: 402,
    retryAfterSeconds: null,
  });

  const rateLimited = await runLiveJob({
    job: "provider-check",
    env: { GOODFETCH_LIVE_PROVIDER_CHECK: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    trials: [{ model: "openai/gpt-5.4-nano", count: 1, maxInputTokens: 500, maxOutputTokens: 200 }],
    writeLog: () => {},
    execute: async (ctx) => ctx.inconclusiveFromGateway({ status: 429, retryAfterSeconds: 12 }),
  });
  expect(rateLimited.status).toBe("inconclusive");
  expect(rateLimited.exitCode).toBe(INCONCLUSIVE_EXIT_CODE);

  const exhausted = await runLiveJob({
    job: "provider-check",
    env: { GOODFETCH_LIVE_PROVIDER_CHECK: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    trials: [{ model: "openai/gpt-5.4-nano", count: 1, maxInputTokens: 500, maxOutputTokens: 200 }],
    writeLog: () => {},
    execute: async (ctx) =>
      ctx.inconclusiveFromGateway({
        status: 402,
        errorCode: "quota_for_entity_exceeded",
      }),
  });
  expect(exhausted.status).toBe("inconclusive");
  expect(exhausted.exitCode).toBe(INCONCLUSIVE_EXIT_CODE);
});

test("reports inconclusive when a planned model is not in the gateway free tier", async () => {
  const outcome = await runLiveJob({
    job: "eval-weekly",
    env: { GOODFETCH_LIVE_EVAL_WEEKLY: "1", AI_GATEWAY_API_KEY: FAKE_GATEWAY_KEY },
    trials: [
      {
        model: "anthropic/claude-sonnet-4.5",
        count: 3,
        maxInputTokens: 1_000,
        maxOutputTokens: 500,
      },
    ],
    writeLog: () => {},
    execute: async () => "should not run",
  });
  expect(outcome.status).toBe("inconclusive");
  expect(outcome.exitCode).toBe(INCONCLUSIVE_EXIT_CODE);
  expect(outcome.callsMade).toBe(0);
});

test("builds isolated environments for Claude Code and Codex without nonessential traffic", () => {
  const claude = claudeCodeLiveEnv({ apiKey: FAKE_GATEWAY_KEY, maxBudgetUsd: 2 });
  expect(claude["CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"]).toBe("1");
  expect(claude["DISABLE_AUTOUPDATER"]).toBe("1");
  expect(claude["DISABLE_TELEMETRY"]).toBe("1");
  expect(claude["DISABLE_ERROR_REPORTING"]).toBe("1");
  expect(claude["ANTHROPIC_BASE_URL"]).toBe("https://ai-gateway.vercel.sh");
  expect(claude["GOODFETCH_MAX_BUDGET_USD"]).toBe("2");

  const codex = codexLiveEnv({ apiKey: FAKE_GATEWAY_KEY });
  expect(codex["CODEX_DISABLE_UPDATE_CHECK"]).toBe("1");
  expect(codex["CODEX_DISABLE_ANALYTICS"]).toBe("1");
  expect(codex["DO_NOT_TRACK"]).toBe("1");
  expect(codex["OPENAI_BASE_URL"]).toBe("https://ai-gateway.vercel.sh/v1");

  const config = codexIsolatedConfigToml("https://ai-gateway.vercel.sh/v1");
  expect(config).toContain("check_for_update_on_startup = false");
  expect(config).toContain("enabled = false");
  expect(config).toContain('env_key = "AI_GATEWAY_API_KEY"');
});

test("estimates worst case accurately and tracks currency limits monotonically", () => {
  const caps = loadCapsTable();
  const est = estimateWorstCaseUsd(
    "recorder",
    [{ model: "openai/gpt-5.4-nano", count: 10, maxInputTokens: 10_000, maxOutputTokens: 2_000 }],
    caps,
  );
  // 10 * (10_000 * 0.20 / 1e6 + 2_000 * 1.25 / 1e6) = 10 * (0.002 + 0.0025) = 0.045
  expect(est.worstCaseUsd).toBeCloseTo(0.045, 9);
  expect(est.exceedsCap).toBe(false);

  const tracker = new CurrencyLimitTracker(0.5);
  expect(tracker.canAfford(0.3)).toBe(true);
  expect(tracker.recordSpend(0.3)).toBe(true);
  expect(tracker.canAfford(0.25)).toBe(false);
  expect(tracker.recordSpend(0.25)).toBe(false);
  expect(tracker.spentUsd).toBeCloseTo(0.3, 9);
});

test("cli live-gate exits at step 1 without any call when repository variable is unset", () => {
  const cliPath = fileURLToPath(new URL("./cli.ts", import.meta.url));
  const result = spawnSync(process.execPath, [cliPath, "live-gate", "--job", "provider-check"], {
    env: { PATH: process.env["PATH"] },
    encoding: "utf8",
  });
  expect(result.status).toBe(0);
  const parsed: unknown = JSON.parse(result.stdout.trim());
  expect(parsed).toEqual({
    job: "provider-check",
    status: "disabled",
    variable: "GOODFETCH_LIVE_PROVIDER_CHECK",
    calls: 0,
  });
});
