// Integration test (spec #152, issue 177): verifies reference host gates,
// explicit downgrade vs refusal, publication classification emitted directly in
// `run.json`, and `tc netem` network profile calibration + time-metric
// invalidation against a real `bench run` on the Proving ground.
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import {
  assessHost,
  classifyForPublication,
  loadHostClasses,
  type HostSnapshot,
} from "../src/host.ts";
import { runCampaign } from "../src/runner.ts";
import { bench, written } from "./bench.ts";

describe(
  "reference host class and published runs",
  { tags: ["story:2-8", "story:2-9", "story:2-10"] },
  () => {
    test.override("provingGroundSeed", 152);

    test("run.json emits publication classification and invalidates time metrics when network calibration drifts", async ({
      provingGround,
    }) => {
      const ran = await bench("stubs.json", provingGround.address);
      expect(ran.stderr).toBe("");
      const { record } = await written(ran.out);
      const classes = loadHostClasses();

      // run.json itself emits publication classification from classifyForPublication.
      expect(record.host.publication.publishable).toBe(true);
      expect(
        record.host.publication.publishable && record.host.publication.publish_as === "reference",
      ).toBe(record.host.class_granted === "reference");

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

      // Campaign with out-of-tolerance network_profile invalidates time metrics while preserving correctness counts.
      const dir = await mkdtemp(join(tmpdir(), "goodfetch-netcal-"));
      try {
        const stubPath = fileURLToPath(new URL("./stub-adapter.ts", import.meta.url));
        const campaignPath = join(dir, "netcal.json");
        await writeFile(
          campaignPath,
          JSON.stringify({
            id: "netcal-drifted",
            host_class: { requested: "local-dev", allow_downgrade: false },
            order_seed: 177,
            blocks: 1,
            warmup: 0,
            iterations: 2,
            concurrency: 1,
            network_profile: {
              mechanism: "tc_netem",
              declared_rtt_ms: 100,
              declared_loss_percent: 0,
              measured_rtt_ms: 130,
              tolerance_ms: 5,
            },
            timeout_ms: 10000,
            max_duration_ms: 60000,
            max_restarts: 1,
            adapters: [{ id: "stub", command: ["node", stubPath, "--delay-ms", "10"] }],
            tasks: ["catalog/2"],
          }),
        );
        const drifted = await runCampaign({
          campaignPath,
          provingGround: provingGround.address,
          out: dir,
        });
        expect(drifted.record.network_calibration).toMatchObject({
          mechanism: "tc_netem",
          within_tolerance: false,
          time_metrics_valid: false,
          correctness_metrics_valid: true,
        });
        const cell = drifted.record.cells[0];
        if (cell === undefined) {
          throw new Error("expected cell in drifted run");
        }
        expect(cell.counts.verified).toBe(3);
        expect(cell.metrics.verified_tasks_per_second.value).toBeNull();
        expect(cell.metrics.task_p50_ms_verified.value).toBeNull();
        expect(cell.metrics.task_p95_ms_verified.value).toBeNull();
        expect(cell.metrics.startup_ms.value).toBeNull();
      } finally {
        await rm(dir, { recursive: true, force: true });
      }
    });
  },
);
