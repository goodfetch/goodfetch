// The `bench` command line, the harness's output writer: it prints one JSON
// line naming the run's folder and status, or `error[<code>]: <message>` on
// stderr and exits 1. Usage:
//
//   node bench/src/cli.ts run <campaign.json> --proving-ground <address> --out <folder>
//   node bench/src/cli.ts live-gate --job <job> [--models <m1,m2>] [--trials <n>]
//     [--max-input-tokens <n>] [--max-output-tokens <n>]
import { parseArgs } from "node:util";
import { BenchError } from "./errors.ts";
import {
  estimateWorstCaseUsd,
  formatRunPlan,
  INCONCLUSIVE_EXIT_CODE,
  isJobEnabled,
  isLiveJobId,
  loadCapsTable,
  type PlannedTrial,
} from "./live-gate.ts";
import { loadModelsManifest, runProviderCheck } from "./provider-check.ts";
import { runCampaign } from "./runner.ts";

const usage = [
  "usage:",
  "  node bench/src/cli.ts run <campaign.json> --proving-ground <address> --out <folder>",
  "  node bench/src/cli.ts live-gate --job <job> [--models <m1,m2>] [--trials <n>] [--max-input-tokens <n>] [--max-output-tokens <n>]",
  "  node bench/src/cli.ts provider-check [--junit <path>]",
  "",
].join("\n");

function parsePositiveInt(raw: string | undefined, fallback: number, flag: string): number {
  if (raw === undefined) {
    return fallback;
  }
  const value = Number(raw);
  if (!Number.isInteger(value) || value < 0) {
    throw new BenchError("invalid_document", `--${flag} must be a non-negative integer`);
  }
  return value;
}

function runLiveGateCommand(values: {
  readonly job?: string;
  readonly models?: string;
  readonly trials?: string;
  readonly "max-input-tokens"?: string;
  readonly "max-output-tokens"?: string;
}): number {
  const job = values.job;
  if (job === undefined || !isLiveJobId(job)) {
    process.stderr.write(usage);
    return 2;
  }
  const caps = loadCapsTable();
  const enablement = isJobEnabled(job, process.env, caps);
  if (!enablement.enabled) {
    process.stdout.write(
      `${JSON.stringify({ job, status: "disabled", variable: enablement.variable, calls: 0 })}\n`,
    );
    return 0;
  }
  const modelList = (values.models ?? "openai/gpt-5.4-nano")
    .split(",")
    .map((m) => m.trim())
    .filter((m) => m.length > 0);
  const count = parsePositiveInt(values.trials, 1, "trials");
  const maxInputTokens = parsePositiveInt(values["max-input-tokens"], 4_000, "max-input-tokens");
  const maxOutputTokens = parsePositiveInt(values["max-output-tokens"], 1_000, "max-output-tokens");
  const planned: PlannedTrial[] = modelList.map((model) => ({
    model,
    count,
    maxInputTokens,
    maxOutputTokens,
  }));
  const estimate = estimateWorstCaseUsd(job, planned, caps);
  process.stderr.write(`${formatRunPlan(estimate)}\n`);
  if (estimate.nonFreeTierModels.length > 0) {
    process.stdout.write(
      `${JSON.stringify({
        job,
        status: "inconclusive",
        reason: "model_not_in_free_tier",
        nonFreeTierModels: estimate.nonFreeTierModels,
      })}\n`,
    );
    return INCONCLUSIVE_EXIT_CODE;
  }
  if (estimate.exceedsCap) {
    process.stdout.write(
      `${JSON.stringify({
        job,
        status: "refused_over_cap",
        worstCaseUsd: estimate.worstCaseUsd,
        capUsd: estimate.capUsd,
      })}\n`,
    );
    return 1;
  }
  process.stdout.write(
    `${JSON.stringify({
      job,
      status: "ready",
      worstCaseUsd: estimate.worstCaseUsd,
      capUsd: estimate.capUsd,
      trials: estimate.totalTrials,
      models: estimate.models,
    })}\n`,
  );
  return 0;
}

async function main(argv: readonly string[]): Promise<number> {
  const { values, positionals } = parseArgs({
    args: [...argv],
    allowPositionals: true,
    options: {
      "proving-ground": { type: "string" },
      out: { type: "string" },
      job: { type: "string" },
      models: { type: "string" },
      trials: { type: "string" },
      "max-input-tokens": { type: "string" },
      "max-output-tokens": { type: "string" },
      junit: { type: "string" },
    },
  });
  const [command, campaignPath] = positionals;
  try {
    if (command === "live-gate") {
      return runLiveGateCommand(values);
    }
    if (command === "provider-check") {
      const outcome = await runProviderCheck({
        recordings: [],
        manifestModels: loadModelsManifest(),
        ...(values.junit !== undefined ? { junitPath: values.junit } : {}),
        sendExchange: async () => {
          throw new BenchError("oracle_refused", "live gateway HTTP transport requires M4 (#425)");
        },
        listModels: async () => [],
      });
      process.stdout.write(`${JSON.stringify(outcome)}\n`);
      return outcome.exitCode;
    }
    const address = values["proving-ground"];
    const out = values.out;
    if (
      command !== "run" ||
      campaignPath === undefined ||
      address === undefined ||
      out === undefined
    ) {
      process.stderr.write(usage);
      return 2;
    }
    const { directory, record } = await runCampaign({ campaignPath, provingGround: address, out });
    process.stdout.write(`${JSON.stringify({ run: directory, status: record.status })}\n`);
    return 0;
  } catch (error) {
    if (error instanceof BenchError) {
      process.stderr.write(`error[${error.code}]: ${error.message}\n`);
      return 1;
    }
    throw error;
  }
}

process.exitCode = await main(process.argv.slice(2));
