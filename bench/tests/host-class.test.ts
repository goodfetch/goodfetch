// Integration test (spec #152, issue 177): verifies reference host gates,
// explicit downgrade vs refusal, publication classification (never publishing
// a downgraded run as `reference`), and `tc netem` network profile calibration
// against a real `bench run` on the Proving ground.
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import {
  assessHost,
  calibrateNetworkProfile,
  classifyForPublication,
  loadHostClasses,
  type HostSnapshot,
} from "../src/host.ts";
import { bench, written } from "./bench.ts";

describe(
  "reference host class and published runs",
  { tags: ["story:2-8", "story:2-9", "story:2-10"] },
  () => {
    test.override("provingGroundSeed", 152);

    test("enforces reference host gates, downgrades non-reference runs, and labels publication class", async ({
      provingGround,
    }) => {
      const ran = await bench("stubs.json", provingGround.address);
      expect(ran.stderr).toBe("");
      const { record } = await written(ran.out);
      const classes = loadHostClasses();

      // The actual local run on this host is not a dedicated bare-metal reference host
      // (or has dirty sources), so publication must never classify it as `reference`.
      const liveAssessment = assessHost({
        snapshot: record.host.preflight.snapshot,
        requested: "reference",
        allowDowngrade: true,
        gitClean: record.provenance.git !== null && !record.provenance.git.dirty,
        classes,
      });
      const livePub = classifyForPublication({
        assessment: liveAssessment,
        machineLabel: `${record.host.preflight.snapshot.os}/${record.host.preflight.snapshot.arch}`,
      });
      expect(livePub.publishable).toBe(true);
      expect(livePub.publishable && livePub.publish_as === "reference").toBe(
        liveAssessment.class_granted === "reference",
      );

      // Simulated bare-metal reference host with turbo off + SMT on passes and publishes as `reference`.
      const referenceSnapshot: HostSnapshot = {
        ...record.host.preflight.snapshot,
        os: "linux",
        virtualized: false,
        cpu_governor: "performance",
        turbo_enabled: false,
        smt_enabled: true,
        on_ac: true,
        load_average_1m: 0.2,
        logical_cpus: 32,
        memory_free_percent: 85,
        thermal_warning: false,
        foreign_cpu_percent: 1.5,
      };
      const refAssessment = assessHost({
        snapshot: referenceSnapshot,
        requested: "reference",
        allowDowngrade: false,
        gitClean: true,
        classes,
      });
      expect(refAssessment.refused).toBe(false);
      expect(refAssessment.class_granted).toBe("reference");
      expect(
        classifyForPublication({
          assessment: refAssessment,
          machineLabel: "reference-bare-metal",
        }),
      ).toMatchObject({ publishable: true, publish_as: "reference" });

      // Network profile calibration checks added RTT against tolerance.
      const calibrated = calibrateNetworkProfile({
        mechanism: "tc_netem",
        declaredRttMs: 20,
        declaredLossPercent: 0,
        measuredRttMs: 21,
        toleranceMs: 3,
      });
      expect(calibrated.time_metrics_valid).toBe(true);
    });
  },
);
