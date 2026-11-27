// The `bench` command line, the harness's output writer: it prints one JSON
// line naming the run's folder and status, or `error[<code>]: <message>` on
// stderr and exits 1. Usage:
//
//   node bench/src/cli.ts run <campaign.json> --proving-ground <address> --out <folder>
//   node bench/src/cli.ts live-gate --job <job> [--models <m1,m2>] [--trials <n>]
//     [--max-input-tokens <n>] [--max-output-tokens <n>]
//   node bench/src/cli.ts provider-check [--junit <path>]
//   node bench/src/cli.ts eval-pr|eval-nightly|eval-weekly [--proving-ground <address>]
//   node bench/src/cli.ts skills
import { parseArgs } from "node:util";
import { BenchError } from "./errors.ts";
import {
  EVALUATION_SIZES,
  runEvaluation,
  type EvaluationSize,
  type EvaluationTask,
} from "./evaluations.ts";
import { evaluateLiveGate, isLiveJobId, loadCapsTable, type PlannedTrial } from "./live-gate.ts";
import {
  createGatewayTransport,
  discoverRecordings,
  loadModelsManifest,
  runProviderCheck,
} from "./provider-check.ts";
import { runCampaign } from "./runner.ts";
import { createPromptfooTrialRunner, runSkillEvaluations } from "./skill-evals.ts";

const usage = [
  "usage:",
  "  node bench/src/cli.ts run <campaign.json> --proving-ground <address> --out <folder>",
  "  node bench/src/cli.ts live-gate --job <job> [--models <m1,m2>] [--trials <n>] [--max-input-tokens <n>] [--max-output-tokens <n>]",
  "  node bench/src/cli.ts provider-check [--junit <path>]",
  "  node bench/src/cli.ts eval-pr|eval-nightly|eval-weekly",
  "  node bench/src/cli.ts skills",
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
  readonly job?: string | undefined;
  readonly models?: string | undefined;
  readonly trials?: string | undefined;
  readonly "max-input-tokens"?: string | undefined;
  readonly "max-output-tokens"?: string | undefined;
}): number {
  const job = values.job;
  if (job === undefined || !isLiveJobId(job)) {
    process.stderr.write(usage);
    return 2;
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

  const decision = evaluateLiveGate({
    job,
    trials: planned,
    writeLog: (line) => {
      if (!line.includes("disabled:")) {
        process.stderr.write(`${line}\n`);
      }
    },
  });

  if (decision.status === "disabled") {
    process.stdout.write(
      `${JSON.stringify({ job, status: "disabled", variable: decision.variable, calls: 0 })}\n`,
    );
    return 0;
  }
  if (decision.status === "inconclusive") {
    process.stdout.write(
      `${JSON.stringify({
        job,
        status: "inconclusive",
        reason: "model_not_in_free_tier",
        nonFreeTierModels: decision.estimate.nonFreeTierModels,
      })}\n`,
    );
    return decision.exitCode;
  }
  if (decision.status === "refused_over_cap") {
    process.stdout.write(
      `${JSON.stringify({
        job,
        status: "refused_over_cap",
        worstCaseUsd: decision.estimate.worstCaseUsd,
        capUsd: decision.estimate.capUsd,
      })}\n`,
    );
    return 1;
  }
  process.stdout.write(
    `${JSON.stringify({
      job,
      status: "ready",
      worstCaseUsd: decision.estimate.worstCaseUsd,
      capUsd: decision.estimate.capUsd,
      trials: decision.estimate.totalTrials,
      models: decision.estimate.models,
    })}\n`,
  );
  return 0;
}

function isEvaluationSize(value: string | undefined): value is EvaluationSize {
  return value === "eval-pr" || value === "eval-nightly" || value === "eval-weekly";
}

function defaultTasksForSize(size: EvaluationSize): readonly EvaluationTask[] {
  const split = EVALUATION_SIZES[size].seedSplit;
  const baseSeed = split === "development" ? 100 : 900;
  return [
    {
      id: `${size}-plans-and-repair`,
      capability: "plans-and-repair",
      seedSplit: split,
      seed: baseSeed + 1,
    },
    { id: `${size}-extraction`, capability: "extraction", seedSplit: split, seed: baseSeed + 2 },
  ];
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
      const caps = loadCapsTable();
      const apiKey = process.env[caps.apiKeyEnv]?.trim() ?? "";
      const transport = createGatewayTransport(caps.gateway, apiKey);
      const outcome = await runProviderCheck({
        recordings: discoverRecordings(),
        manifestModels: loadModelsManifest(),
        junitPath: values.junit,
        sendExchange: transport.sendExchange,
        listModels: transport.listModels,
      });
      process.stdout.write(`${JSON.stringify(outcome)}\n`);
      return outcome.exitCode;
    }
    if (isEvaluationSize(command)) {
      const caps = loadCapsTable();
      const outcome = await runEvaluation({
        size: command,
        tasks: defaultTasksForSize(command),
        provenance: {
          commit: process.env["GITHUB_SHA"] ?? "HEAD",
          gateway: caps.gateway,
          providerFingerprint: null,
          localServerVersion: null,
          localModelDigest: null,
          seed: EVALUATION_SIZES[command].seedSplit === "development" ? 100 : 900,
          promptSnapshotHashes: {},
        },
        runTrial: async () => {
          throw new BenchError(
            "oracle_refused",
            "live evaluation trial execution requires a running Proving ground (--proving-ground)",
          );
        },
      });
      process.stdout.write(`${JSON.stringify(outcome)}\n`);
      return outcome.exitCode;
    }
    if (command === "skills") {
      const outcome = await runSkillEvaluations({
        products: ["collie", "foxhound"],
        agents: ["claude-code", "codex"],
        tierModels: { small: "openai/gpt-5.4-nano", mid: null, large: "openai/gpt-5.2" },
        runAgentTrial: createPromptfooTrialRunner(),
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
