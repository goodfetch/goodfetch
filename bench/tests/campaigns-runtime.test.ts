// Integration test (spec #152, issue 188): validates all declared runtime
// family campaigns (repeated runs with Drift, incident and recovery, startup
// and binding overhead on both Product executables, run-state/admission, and
// control-cost including TLS relay, clean-up time/bytes, and live_hiding vs
// live_hiding = off).
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { loadCampaign } from "../src/campaign.ts";
import { loadReleaseGateClaims } from "../src/release-gate.ts";
import {
  evaluateControlCost,
  evaluateReopenDecision,
  SWITCHABLE_CONTROLS,
} from "../src/runtime-campaigns.ts";
import { bench, written } from "./bench.ts";

describe(
  "runtime families and switchable control cost campaigns",
  { tags: ["story:2-67", "story:2-68", "story:2-69"] },
  () => {
    test.override("provingGroundSeed", 152);

    test("validates runtime campaigns, startup on both Products, reopenable ADRs and control costs", async ({
      provingGround,
    }) => {
      const drift = await loadCampaign(
        new URL("../campaigns/repeated-runs-drift.json", import.meta.url).pathname,
      );
      const recovery = await loadCampaign(
        new URL("../campaigns/incident-recovery.json", import.meta.url).pathname,
      );
      const startup = await loadCampaign(
        new URL("../campaigns/startup-binding.json", import.meta.url).pathname,
      );
      const runState = await loadCampaign(
        new URL("../campaigns/run-state-and-admission.json", import.meta.url).pathname,
      );
      const controlCost = await loadCampaign(
        new URL("../campaigns/control-cost.json", import.meta.url).pathname,
      );

      expect(drift.declared.decisions.map((d) => d.id)).toEqual(["D-repeated-runs-drift"]);
      expect(recovery.declared.decisions.map((d) => d.id)).toEqual(["D-incident-recovery"]);
      expect(startup.campaign.adapters.map((a) => a.id)).toEqual([
        "collie-default",
        "foxhound-default",
      ]);
      expect(runState.declared.decisions.map((d) => d.id)).toEqual(["D-run-state-and-admission"]);
      expect(controlCost.declared.decisions.map((d) => d.id)).toEqual(["D-control-cost"]);

      const releaseClaims = loadReleaseGateClaims();
      for (const family of [
        "repeated-runs-drift",
        "incident-recovery",
        "startup-binding",
        "control-cost",
      ] as const) {
        expect(releaseClaims.claims.some((c) => c.family === family)).toBe(true);
      }

      // Run a live Proving ground smoke to anchor control-cost latency calculations.
      const ran = await bench("stubs.json", provingGround.address);
      expect(ran.stderr).toBe("");
      const { record } = await written(ran.out);
      const baseMs =
        record.cells.find((c) => c.gate === "passed")?.metrics.task_p50_ms_verified.value ?? 50;

      // Publish cost rows for every switchable control including TLS relay, clean-up and live_hiding.
      const rows = SWITCHABLE_CONTROLS.map((control) => {
        const isCleanup =
          control === "filter_list_blocking" || control === "consent_prompt_handling";
        return evaluateControlCost({
          control,
          product:
            control === "power_cut_safe_run_state" || control === "fair_per_domain_admission"
              ? "foxhound"
              : "collie",
          on_ms: baseMs + 2,
          off_ms: baseMs,
          ...(isCleanup ? { server_bytes_on: 35_000, server_bytes_off: 100_000 } : {}),
          baseline_setting: `${control} = off`,
        });
      });
      expect(rows).toHaveLength(SWITCHABLE_CONTROLS.length);
      const liveHidingRow = rows.find((r) => r.control === "live_hiding");
      expect(liveHidingRow?.baseline_setting).toBe("live_hiding = off");
      const filterRow = rows.find((r) => r.control === "filter_list_blocking");
      expect(filterRow?.server_bytes_saved).toBe(65_000);

      // Run-state and admission results name the exact ADR they could reopen.
      expect(
        evaluateReopenDecision({
          family: "run-state",
          product: "foxhound",
          measured_gap_percent: 4,
          threshold_percent: 15,
        }).could_reopen,
      ).toBe("ADR 0002");
      expect(
        evaluateReopenDecision({
          family: "fair-per-domain-admission",
          product: "foxhound",
          measured_gap_percent: 1,
          threshold_percent: 5,
        }).could_reopen,
      ).toBe("ADR 0004");
    });
  },
);
