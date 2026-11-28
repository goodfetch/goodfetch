// Integration test (spec #152, issue 188): validates all declared runtime
// family campaigns (repeated runs with Drift, incident and recovery, startup
// and binding overhead on both Product executables with owned tasks,
// run-state/admission with the 2026-11-07 Foxhound amendment, and control-cost
// with distinct per-arm settings), then executes a real control-cost and
// reopen-decision campaign against Proving ground measuring REAL latency
// deltas and REAL Proving ground `/observations` server-side bytes.
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import { loadCampaign } from "../src/campaign.ts";
import { loadReleaseGateClaims } from "../src/release-gate.ts";
import { runCampaign } from "../src/runner.ts";

function distinctCommands(
  adapters: readonly { readonly id: string; readonly command: readonly string[] }[],
): boolean {
  const seen = new Set(adapters.map((a) => JSON.stringify(a.command)));
  return seen.size === adapters.length;
}

describe(
  "runtime families and switchable control cost campaigns",
  { tags: ["story:2-67", "story:2-68", "story:2-69"] },
  () => {
    test.override("provingGroundSeed", 152);

    test("validates declared runtime campaigns and measures real Proving ground latency and /observations bytes", async ({
      provingGround,
    }) => {
      const drift = await loadCampaign(
        fileURLToPath(new URL("../campaigns/repeated-runs-drift.json", import.meta.url)),
      );
      const recovery = await loadCampaign(
        fileURLToPath(new URL("../campaigns/incident-recovery.json", import.meta.url)),
      );
      const startup = await loadCampaign(
        fileURLToPath(new URL("../campaigns/startup-binding.json", import.meta.url)),
      );
      const runState = await loadCampaign(
        fileURLToPath(new URL("../campaigns/run-state-and-admission.json", import.meta.url)),
      );
      const controlCost = await loadCampaign(
        fileURLToPath(new URL("../campaigns/control-cost.json", import.meta.url)),
      );

      // Every paired campaign uses real family tasks and distinct CLI/setting flags per arm.
      expect(drift.campaign.tasks).toEqual(["schedule/1"]);
      expect(distinctCommands(drift.campaign.adapters)).toBe(true);
      expect(recovery.campaign.tasks).toEqual(["incidents/1"]);
      expect(distinctCommands(recovery.campaign.adapters)).toBe(true);
      expect(startup.campaign.adapters.map((a) => ({ id: a.id, tasks: a.tasks }))).toEqual([
        { id: "collie-default", tasks: ["catalog/2"] },
        { id: "foxhound-default", tasks: ["catalog/1"] },
      ]);
      // 2026-11-07 Foxhound amendment: batch/full/off and fewest_in_flight/round_robin.
      expect(runState.campaign.adapters.map((a) => a.id)).toEqual([
        "foxhound-batch",
        "foxhound-full",
        "foxhound-off",
        "foxhound-fewest-in-flight",
        "foxhound-round-robin",
      ]);
      expect(distinctCommands(runState.campaign.adapters)).toBe(true);
      expect(distinctCommands(controlCost.campaign.adapters)).toBe(true);

      // Release-gate claims cover run-state-and-admission AND both Products for startup-binding.
      const releaseClaims = loadReleaseGateClaims();
      for (const family of [
        "repeated-runs-drift",
        "incident-recovery",
        "startup-binding",
        "run-state-and-admission",
        "control-cost",
      ] as const) {
        expect(releaseClaims.claims.some((c) => c.family === family)).toBe(true);
      }
      expect(
        releaseClaims.claims.some((c) => c.family === "startup-binding" && c.product === "collie"),
      ).toBe(true);
      expect(
        releaseClaims.claims.some(
          (c) => c.family === "startup-binding" && c.product === "foxhound",
        ),
      ).toBe(true);

      // Execute a real Proving ground campaign wired to runner.ts control_costs + reopen_decisions + /observations bytes.
      const dir = await mkdtemp(join(tmpdir(), "goodfetch-runtime-"));
      try {
        const stubPath = fileURLToPath(new URL("./stub-adapter.ts", import.meta.url));
        const campaignPath = join(dir, "runtime-live.json");
        await writeFile(
          campaignPath,
          JSON.stringify({
            id: "runtime-live-check",
            host_class: { requested: "local-dev", allow_downgrade: false },
            order_seed: 188,
            blocks: 1,
            warmup: 0,
            iterations: 3,
            concurrency: 1,
            timeout_ms: 10000,
            max_duration_ms: 60000,
            max_restarts: 1,
            tasks: ["catalog/2"],
            control_cost_comparisons: [
              {
                control: "filter_list_blocking",
                product: "collie",
                on_adapter: "control-on",
                off_adapter: "control-off",
                baseline_setting: "filter_lists = off",
                measure_server_bytes: true,
              },
              {
                control: "live_hiding",
                product: "collie",
                on_adapter: "control-on",
                off_adapter: "control-off",
                baseline_setting: "live_hiding = off",
              },
            ],
            reopen_checks: [
              {
                family: "run-state",
                product: "foxhound",
                candidate_adapter: "control-off",
                baseline_adapter: "control-on",
                threshold_percent: 15,
              },
              {
                family: "fair-per-domain-admission",
                product: "foxhound",
                candidate_adapter: "control-off",
                baseline_adapter: "control-on",
                threshold_percent: 5,
              },
            ],
            adapters: [
              { id: "control-on", command: ["node", stubPath, "--delay-ms", "10"] },
              {
                id: "control-off",
                command: ["node", stubPath, "--delay-ms", "30", "--extra-fetches", "2"],
              },
            ],
          }),
        );

        const { record } = await runCampaign({
          campaignPath,
          provingGround: provingGround.address,
          out: dir,
        });

        // Real server-side wire bytes observed by Proving ground /observations.
        const bytesOn = record.server_bytes_by_adapter["control-on"] ?? 0;
        const bytesOff = record.server_bytes_by_adapter["control-off"] ?? 0;
        expect(bytesOn).toBeGreaterThan(0);
        expect(bytesOff).toBeGreaterThan(bytesOn);

        const filterRow = record.control_costs.find((r) => r.control === "filter_list_blocking");
        expect(filterRow?.server_bytes_saved).toBe(bytesOff - bytesOn);
        expect(filterRow?.server_bytes_saved).toBeGreaterThan(0);

        const liveHidingRow = record.control_costs.find((r) => r.control === "live_hiding");
        expect(liveHidingRow?.baseline_setting).toBe("live_hiding = off");
        expect(liveHidingRow?.server_bytes_saved).toBeNull();

        expect(record.reopen_decisions.find((r) => r.family === "run-state")?.could_reopen).toBe(
          "ADR 0002",
        );
        expect(
          record.reopen_decisions.find((r) => r.family === "fair-per-domain-admission")
            ?.could_reopen,
        ).toBe("ADR 0004");
      } finally {
        await rm(dir, { recursive: true, force: true });
      }
    });
  },
);
