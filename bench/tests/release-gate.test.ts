// Integration test (spec #152, issue 176): runs a real `bench run` against
// the commit's Proving ground, feeds the resulting cells into `node bench/src/cli.ts release-gate`
// across both Products, checks pass (exit 0), block (exit 1), malformed input
// validation (exit 1), host-gate refusal (exit 75 INCONCLUSIVE), computed
// behind-competitor rows, Foxhound-only block, and `--mode patch-correctness-only`.
import { execFile } from "node:child_process";
import { mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { test } from "@goodfetch/test-support";
import { describe, expect } from "vitest";
import {
  loadReleaseGateClaims,
  type FamilyBenchmarkRow,
  type ReleaseGateDecision,
  type ReleaseGateInput,
} from "../src/release-gate.ts";
import { bench, written } from "./bench.ts";

const exec = promisify(execFile);
const cliPath = fileURLToPath(new URL("../src/cli.ts", import.meta.url));

async function runReleaseGateCli(
  inputPath: string,
  extraArgs: readonly string[] = [],
): Promise<{ readonly code: number; readonly stdout: string; readonly stderr: string }> {
  try {
    const { stdout, stderr } = await exec(process.execPath, [
      cliPath,
      "release-gate",
      inputPath,
      ...extraArgs,
    ]);
    return { code: 0, stdout, stderr };
  } catch (error) {
    if (error instanceof Error && "code" in error && typeof error.code === "number") {
      return {
        code: error.code,
        stdout: "stdout" in error ? String(error.stdout) : "",
        stderr: "stderr" in error ? String(error.stderr) : "",
      };
    }
    throw error;
  }
}

describe(
  "release-gate decision",
  {
    tags: [
      "story:2-58",
      "story:2-59",
      "story:2-60",
      "story:2-61",
      "story:2-62",
      "story:2-63",
      "story:2-86",
      "story:2-87",
    ],
  },
  () => {
    test.override("provingGroundSeed", 152);

    test("executes bench release-gate CLI for pass, block, malformed input, patch mode and exit 75 inconclusive", async ({
      provingGround,
    }) => {
      const ran = await bench("decisions.json", provingGround.address);
      expect(ran.stderr).toBe("");
      const { record } = await written(ran.out);
      const claimsFile = loadReleaseGateClaims();

      const fastCell = record.cells.find((c) => c.adapter === "fast");
      if (fastCell === undefined) {
        throw new Error("expected fast cell in decisions.json run");
      }

      const collieP50 = fastCell.metrics.task_p50_ms_verified.value ?? 50;
      const candidateRow: FamilyBenchmarkRow = {
        family: "extraction",
        product: "collie",
        adapter: fastCell.adapter,
        is_competitor: false,
        gate_passed: fastCell.gate === "passed",
        verified: fastCell.counts.verified,
        false_success: fastCell.counts.false_success,
        failed: fastCell.counts.failed,
        incomplete: fastCell.counts.incomplete,
        timeout: fastCell.counts.timeout,
        crashed: fastCell.counts.crashed,
        primary_metric_value: collieP50,
      };
      // Competitor that is 20% faster (lower latency) so behind_competitors is computed from candidate_rows.
      const fasterCompetitor: FamilyBenchmarkRow = {
        ...candidateRow,
        product: null,
        adapter: "competitor-fast",
        is_competitor: true,
        primary_metric_value: collieP50 * 0.8,
      };

      const dir = await mkdtemp(join(tmpdir(), "goodfetch-rg-"));
      try {
        // 1. Pass with computed behind_competitors -> exit 0
        const passInput: ReleaseGateInput = {
          mode: "release",
          claims: claimsFile.claims,
          candidate_rows: [candidateRow, fasterCompetitor],
          previous_release_rows: [candidateRow],
          behind_competitors: [
            {
              product: "collie",
              family: "extraction",
              competitor: "competitor-fast",
              metric: "task_p50_ms_verified",
              gap: "+25.0%",
              measured_cause: "extra schema validation pass",
              next_cut: "reuse compiled validator across pages",
            },
          ],
          host: {
            host_class: record.host.class_granted,
            host_label: "local-controlled test host",
            refused_by_host_gates: false,
          },
        };
        const passFile = join(dir, "pass.json");
        await writeFile(passFile, JSON.stringify(passInput));
        const passRes = await runReleaseGateCli(passFile);
        expect(passRes.code).toBe(0);
        const passDecision: ReleaseGateDecision = JSON.parse(passRes.stdout);
        expect(passDecision.decision).toBe("pass");
        expect(passDecision.live_check.tier).toBe("live");
        expect(passDecision.live_check.status).toBe("pass");
        expect(passDecision.behind_competitors).toHaveLength(1);
        expect(passDecision.behind_competitors[0]?.competitor).toBe("competitor-fast");
        expect(passDecision.behind_competitors[0]?.gap).toBe("+25.0%");

        // 2. Host-gate refusal -> exit 75 (INCONCLUSIVE, never 0!)
        const refusedFile = join(dir, "refused.json");
        await writeFile(
          refusedFile,
          JSON.stringify({
            ...passInput,
            host: {
              host_class: "local-controlled",
              host_label: "busy host",
              refused_by_host_gates: true,
              refusal_reason: "load_per_cpu_1m exceeded limit",
            },
          }),
        );
        const refusedRes = await runReleaseGateCli(refusedFile);
        expect(refusedRes.code).toBe(75);
        const refusedDecision: ReleaseGateDecision = JSON.parse(refusedRes.stdout);
        expect(refusedDecision.live_check.status).toBe("inconclusive");

        // 3. Foxhound-only block (Collie passes, Foxhound blocks) -> exit 1
        const approvedFoxClaims = claimsFile.claims.map((c) =>
          c.family === "http-crawl" ? { ...c, delta: 0.1, delta_approved_by_owner: true } : c,
        );
        const foxRow: FamilyBenchmarkRow = {
          ...candidateRow,
          family: "http-crawl",
          product: "foxhound",
          adapter: "foxhound-default",
          primary_metric_value: 100,
        };
        const foxBlockFile = join(dir, "fox-block.json");
        await writeFile(
          foxBlockFile,
          JSON.stringify({
            mode: "release",
            claims: approvedFoxClaims,
            candidate_rows: [candidateRow, { ...foxRow, primary_metric_value: 50 }],
            previous_release_rows: [candidateRow, foxRow],
            host: passInput.host,
          }),
        );
        const foxBlockRes = await runReleaseGateCli(foxBlockFile);
        expect(foxBlockRes.code).toBe(1);
        const foxBlockDecision: ReleaseGateDecision = JSON.parse(foxBlockRes.stdout);
        expect(foxBlockDecision.decision).toBe("block");
        expect(foxBlockDecision.products.collie.decision).toBe("pass");
        expect(foxBlockDecision.products.foxhound.decision).toBe("block");

        // 4. --mode patch-correctness-only overrides release mode and passes metric regressions -> exit 0
        const patchRes = await runReleaseGateCli(foxBlockFile, [
          "--mode",
          "patch-correctness-only",
        ]);
        expect(patchRes.code).toBe(0);

        // 5. Malformed JSON input fails validation with BenchError invalid_document -> exit 1
        const badFile = join(dir, "bad.json");
        await writeFile(badFile, JSON.stringify({ mode: "release" }));
        const badRes = await runReleaseGateCli(badFile);
        expect(badRes.code).toBe(1);
        expect(badRes.stderr).toContain("error[invalid_document]");
      } finally {
        await rm(dir, { recursive: true, force: true });
      }
    });
  },
);
