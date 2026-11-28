import { describe, expect, test } from "vitest";
import type { Attempt } from "./cell.ts";
import { loadPriceTable, parsePriceTable } from "./prices.ts";
import { buildThreeViews, formatRawFigure, PLAN_AMORTIZATION_RUN_COUNTS } from "./views.ts";

function makeAttempt(overrides: Partial<Attempt> = {}): Attempt {
  return {
    attempt_id: "a:t:b0:measure:0",
    cell_id: "a:t:b0",
    adapter: "goodfetch",
    task: "catalog/2",
    block: 0,
    phase: "measure",
    index: 0,
    status: "verified",
    tool_status: "success",
    error_code: null,
    reason: null,
    timing: { queue_ms: 1, task_ms: 100, oracle_ms: 2 },
    generation: 0,
    judgement: null,
    adapter_metrics: null,
    ...overrides,
  };
}

describe("price tables", () => {
  test("loads versioned v1 price table with provenance (source and date)", () => {
    const table = loadPriceTable();
    expect(table.version).toBe(1);
    expect(table.effective_date).toMatch(/^\d{4}-\d{2}-\d{2}$/);
    expect(table.source.length).toBeGreaterThan(0);
    expect(table.hosts["reference-bare-metal"]?.hourly_usd).toBeGreaterThan(0);
    expect(table.hosts["hetzner-ccx53"]?.hourly_usd).toBeGreaterThan(0);
    expect(table.models["openai/gpt-5.4-nano"]?.input_per_1m_usd).toBeGreaterThan(0);
    expect(table.hosted_browsers["browserbase"]?.per_browser_minute_usd).toBeGreaterThan(0);
  });

  test("rejects a price table missing source or date", () => {
    expect(() =>
      parsePriceTable({
        version: 1,
        effective_date: "not-a-date",
        source: "test",
        hosts: {},
        models: {},
        proxies: {},
        captchas: {},
        hosted_browsers: {},
      }),
    ).toThrow(/effective_date/);
  });
});

describe("three views, counts and cost model", () => {
  test("emits raw view always paired with verified, false-success and needs-review counts", () => {
    const prices = loadPriceTable();
    const attempts: Attempt[] = [
      makeAttempt({
        index: 0,
        status: "verified",
        tool_status: "success",
        timing: { queue_ms: 0, task_ms: 100, oracle_ms: 1 },
      }),
      makeAttempt({
        index: 1,
        status: "verified",
        tool_status: "needs-review",
        timing: { queue_ms: 0, task_ms: 120, oracle_ms: 1 },
      }),
      makeAttempt({
        index: 2,
        status: "false_success",
        tool_status: "success",
        timing: { queue_ms: 0, task_ms: 80, oracle_ms: 1 },
      }),
      makeAttempt({
        index: 3,
        status: "failed",
        tool_status: "needs-review",
        timing: { queue_ms: 0, task_ms: 90, oracle_ms: 1 },
      }),
    ];

    const report = buildThreeViews({
      adapter: "goodfetch",
      task: "catalog/2",
      attempts,
      windowMs: 1000,
      hostMachineId: "reference-bare-metal",
      prices,
    });

    expect(report.raw.counts).toEqual({
      verified: 2,
      false_success: 1,
      needs_review: 2,
      needs_review_verified: 1,
      failed: 1,
      total_attempts: 4,
    });
    expect(report.raw.raw_tasks_per_second).toBeCloseTo(4, 6);
    expect(report.per_verified.verified_tasks_per_second).toBeCloseTo(2, 6);

    const formatted = formatRawFigure(report.raw);
    expect(formatted).toContain("verified=2/4");
    expect(formatted).toContain("false_success=1");
    expect(formatted).toContain("needs_review=2");
  });

  test("sums host, model, proxy, CAPTCHA and hosted browser spend including failed attempts", () => {
    const prices = loadPriceTable();
    const attempts: Attempt[] = [
      makeAttempt({
        index: 0,
        status: "verified",
        timing: { queue_ms: 0, task_ms: 1_800_000, oracle_ms: 1 }, // 0.5h
        adapter_metrics: {
          cost_usage: {
            model: { id: "openai/gpt-5.4-nano", input_tokens: 1_000_000, output_tokens: 500_000 }, // 0.10 + 0.20 = 0.30
            proxy: { tier: "residential", bytes: 1_000_000_000 }, // 5.50
            captcha: { kind: "turnstile", solves: 100 }, // 0.299
            hosted_browser: { vendor: "browserbase", minutes: 10 }, // 0.02
            human_seconds: 120,
          },
        },
      }),
      makeAttempt({
        index: 1,
        status: "failed",
        tool_status: "failure",
        timing: { queue_ms: 0, task_ms: 1_800_000, oracle_ms: 1 }, // 0.5h (failed!)
        adapter_metrics: {
          cost_usage: {
            model: { id: "openai/gpt-5.4-nano", input_tokens: 1_000_000, output_tokens: 500_000 }, // 0.30
            proxy: { tier: "residential", bytes: 500_000_000 }, // 2.75
            captcha: { kind: "turnstile", solves: 100 }, // 0.299
            hosted_browser: { vendor: "browserbase", minutes: 5 }, // 0.01
            human_seconds: 60,
          },
        },
      }),
    ];

    const report = buildThreeViews({
      adapter: "goodfetch",
      task: "catalog/2",
      attempts,
      windowMs: 3_600_000, // 1.0 hour total wall time -> $0.42 host cost
      hostMachineId: "reference-bare-metal",
      prices,
    });

    expect(report.cost.available).toBe(true);
    if (!report.cost.available) {
      return;
    }
    expect(report.cost.breakdown.host_usd).toBeCloseTo(0.42, 6);
    expect(report.cost.breakdown.model_usd).toBeCloseTo(0.6, 6);
    expect(report.cost.breakdown.proxy_usd).toBeCloseTo(8.25, 6);
    expect(report.cost.breakdown.captcha_usd).toBeCloseTo(0.598, 6);
    expect(report.cost.breakdown.hosted_browser_usd).toBeCloseTo(0.03, 6);
    const expectedTotal = 0.42 + 0.6 + 8.25 + 0.598 + 0.03;
    expect(report.cost.breakdown.total_usd).toBeCloseTo(expectedTotal, 6);
    expect(report.cost.breakdown.failed_attempt_usd).toBeCloseTo(
      0.21 + 0.3 + 2.75 + 0.299 + 0.01,
      6,
    );
    expect(report.cost.dollars_per_1k_verified).toBeCloseTo(expectedTotal * 1000, 4);

    // Human time is separate and never folded into dollars.
    expect(report.human_time).toEqual({ total_seconds: 180, seconds_per_verified: 180 });
  });

  test("prices another rented machine with public rate and omits cost view for unpriced machine", () => {
    const prices = loadPriceTable();
    const attempts = [makeAttempt({ timing: { queue_ms: 0, task_ms: 3_600_000, oracle_ms: 1 } })];

    const rented = buildThreeViews({
      adapter: "goodfetch",
      task: "catalog/2",
      attempts,
      windowMs: 3_600_000,
      hostMachineId: "hetzner-ccx53",
      prices,
    });
    if (!rented.cost.available) {
      throw new Error("expected rented machine cost to be available");
    }
    expect(rented.cost.breakdown.host_usd).toBeCloseTo(1.12, 6);
    expect(rented.cost.host_pricing.machine_id).toBe("hetzner-ccx53");

    const laptop = buildThreeViews({
      adapter: "goodfetch",
      task: "catalog/2",
      attempts,
      windowMs: 3_600_000,
      hostMachineId: null,
      prices,
    });
    if (laptop.cost.available) {
      throw new Error("expected unpriced machine cost to be unavailable");
    }
    expect(laptop.cost.reason).toContain("no public hourly price");
  });

  test("amortizes Plan one-time cost over 1, 10, 100 and 1,000 runs", () => {
    const prices = loadPriceTable();
    const attempts = [makeAttempt({ timing: { queue_ms: 0, task_ms: 1000, oracle_ms: 1 } })];

    const report = buildThreeViews({
      adapter: "goodfetch",
      task: "catalog/2",
      attempts,
      windowMs: 1000,
      hostMachineId: "reference-bare-metal",
      prices,
      planOneTimeCost: {
        oneTimeUsd: 1.0,
        declaredRunCounts: [...PLAN_AMORTIZATION_RUN_COUNTS],
        verifiedPerRun: 100,
      },
    });

    expect(report.cost.available).toBe(true);
    if (!report.cost.available || report.cost.plan_amortization === null) {
      throw new Error("expected plan amortization");
    }
    expect(report.cost.plan_amortization.map((r) => r.runs)).toEqual([1, 10, 100, 1000]);
    expect(report.cost.plan_amortization[0]?.amortized_per_run_usd).toBeCloseTo(1.0, 6);
    expect(report.cost.plan_amortization[1]?.amortized_per_run_usd).toBeCloseTo(0.1, 6);
    expect(report.cost.plan_amortization[2]?.amortized_per_run_usd).toBeCloseTo(0.01, 6);
    expect(report.cost.plan_amortization[3]?.amortized_per_run_usd).toBeCloseTo(0.001, 6);
  });
});
