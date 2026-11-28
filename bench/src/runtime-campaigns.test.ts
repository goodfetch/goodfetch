import { describe, expect, test } from "vitest";
import {
  evaluateControlCost,
  evaluateReopenDecision,
  SWITCHABLE_CONTROLS,
} from "./runtime-campaigns.ts";

describe("runtime campaigns and control cost reporting", () => {
  test("computes time cost and server-side byte savings for clean-up and live_hiding", () => {
    const filterList = evaluateControlCost({
      control: "filter_list_blocking",
      product: "collie",
      on_ms: 82,
      off_ms: 100,
      server_bytes_on: 40_000,
      server_bytes_off: 100_000,
      baseline_setting: "filter_list_blocking = off",
    });
    expect(filterList.time_delta_ms).toBe(-18);
    expect(filterList.server_bytes_saved).toBe(60_000);
    expect(filterList.server_bytes_saved_percent).toBeCloseTo(60, 6);

    const liveHiding = evaluateControlCost({
      control: "live_hiding",
      product: "collie",
      on_ms: 104,
      off_ms: 100,
      baseline_setting: "live_hiding = off",
    });
    expect(liveHiding.baseline_setting).toBe("live_hiding = off");
    expect(liveHiding.time_delta_ms).toBe(4);
    expect(liveHiding.server_bytes_saved).toBeNull();
  });

  test("covers all switchable controls including TLS relay and reports reopenable ADRs", () => {
    expect(SWITCHABLE_CONTROLS).toContain("tls_relay_cert_exceptions");
    expect(SWITCHABLE_CONTROLS).toContain("live_hiding");
    expect(SWITCHABLE_CONTROLS).toContain("consent_prompt_handling");

    const runState = evaluateReopenDecision({
      family: "run-state",
      product: "foxhound",
      measured_gap_percent: 25,
      threshold_percent: 15,
    });
    expect(runState.could_reopen).toBe("ADR 0002");
    expect(runState.triggers_reopen_review).toBe(true);

    const admission = evaluateReopenDecision({
      family: "fair-per-domain-admission",
      product: "foxhound",
      measured_gap_percent: 2,
      threshold_percent: 5,
    });
    expect(admission.could_reopen).toBe("ADR 0004");
    expect(admission.triggers_reopen_review).toBe(false);
  });
});
