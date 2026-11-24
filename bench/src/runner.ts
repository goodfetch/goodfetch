// A bench run (ADR 0025): a campaign's adapters on the Proving ground's
// tasks, judged by its oracle. The run reads its provenance and the host
// first, then runs each block's adapters in a seeded order; each adapter runs
// each task as a cell: one conformance attempt, the warmup, then the measured
// attempts in a closed loop. A cell that fails its gate is not measured, and
// its plan stays visible as skipped attempts. Carried over from the lab's
// engine, without its open-loop mode and its legacy runner.
import { randomUUID } from "node:crypto";
import { appendFile, mkdir, writeFile } from "node:fs/promises";
import { platform } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { AdapterSlot } from "./adapter.ts";
import { loadCampaign, type Campaign, type CampaignAdapter, type Row } from "./campaign.ts";
import {
  countStatuses,
  mergeUsage,
  summarize,
  type AdapterMetrics,
  type Attempt,
  type Cell,
  type CellRun,
  type Counts,
  type Phase,
} from "./cell.ts";
import { BenchError, errorCode, statusFor, type AttemptStatus } from "./errors.ts";
import {
  assessHost,
  blockNoisy,
  loadHostClasses,
  snapshotHost,
  type Gate,
  type GateResult,
  type HostClass,
  type HostSnapshot,
} from "./host.ts";
import {
  detectProbes,
  metricBook,
  type Availability,
  type MetricBook,
  type Probe,
} from "./metrics.ts";
import { shuffle } from "./order.ts";
import { captureProvenance, changedSources, type Provenance } from "./provenance.ts";
import {
  provingGround,
  type Judgement,
  type ListedTask,
  type ProvingGround,
  type Submission,
  type SubmittedError,
  type ToolStatus,
} from "./proving-ground.ts";
import { sampleProcessTree, type TreeSampler, type TreeUsage } from "./resources.ts";
import { parserOf } from "./schema.ts";

/** The repository's root, whose bench sources the provenance fingerprints. */
const repository = fileURLToPath(new URL("../..", import.meta.url));
const sources = ["bench/src", "bench/hosts", "bench/adapters"] as const;

/** A run's standing: complete, or why its figures cannot be compared. */
export type RunStatus = "complete" | "noisy_host" | "invalid_provenance";

/** The record a run writes to `run.json`. */
export interface RunRecord {
  readonly schema_version: 1;
  readonly run_id: string;
  readonly started_at: string;
  readonly finished_at: string;
  readonly status: RunStatus;
  readonly campaign: { readonly id: string; readonly sha256: string; readonly document: Campaign };
  readonly provenance: Provenance;
  /** The bench sources that changed while the run ran. */
  readonly changed_sources: readonly string[];
  readonly proving_ground: {
    readonly version: string;
    readonly seed: number;
    readonly tasks: readonly { readonly id: string; readonly digest: string }[];
  };
  readonly host: {
    readonly class_requested: HostClass;
    readonly class_granted: HostClass;
    readonly preflight: { readonly snapshot: HostSnapshot; readonly gates: readonly GateResult[] };
    readonly postflight: { readonly snapshot: HostSnapshot };
  };
  readonly metric_availability: Readonly<Record<string, Availability>>;
  readonly blocks: readonly {
    readonly block: number;
    readonly order: readonly string[];
    readonly noisy: boolean;
  }[];
  /** Each adapter's row, and the Product and settings it said it ran with. */
  readonly adapters: readonly AdapterRow[];
  readonly cells: readonly Cell[];
  readonly totals: Counts;
}

/** One adapter as the run ran it: a matched row's marks are its settings (ADRs 0023 and 0025). */
export interface AdapterRow {
  readonly id: string;
  /** The Product it drove, as its `hello` named it (ADR 0094); null for a competitor's tool. */
  readonly product: string | null;
  readonly row: Row;
  /** The settings it answered `prepare` with; null when it answered none or never started. */
  readonly settings: Readonly<Record<string, unknown>> | null;
}

/** What an adapter answers to `run`. */
type RunAnswer =
  | {
      readonly status: ToolStatus;
      readonly records: readonly unknown[];
      readonly error?: SubmittedError;
      readonly metrics?: AdapterMetrics;
    }
  | { readonly status: "unsupported"; readonly reason: string };

const runAnswer = parserOf<RunAnswer>({
  oneOf: [
    {
      type: "object",
      additionalProperties: false,
      required: ["status", "records"],
      properties: {
        status: { enum: ["success", "needs-review", "failure"] },
        records: { type: "array" },
        error: {
          type: "object",
          additionalProperties: false,
          required: ["code"],
          properties: {
            code: { type: "string" },
            network_error: { type: "string" },
            rule: { type: "string" },
            setting: { type: "string" },
          },
        },
        metrics: {
          type: "object",
          additionalProperties: false,
          properties: { cdp_commands: { type: "integer", minimum: 0 } },
        },
      },
    },
    {
      type: "object",
      additionalProperties: false,
      required: ["status", "reason"],
      properties: { status: { const: "unsupported" }, reason: { type: "string" } },
    },
  ],
});

/** What every cell of a run shares. */
interface RunContext {
  readonly campaign: Campaign;
  readonly folder: string;
  readonly ground: ProvingGround;
  readonly probes: ReadonlySet<Probe>;
  readonly book: MetricBook;
  readonly record: (attempt: Attempt) => Promise<void>;
  readonly rows: ReadonlyMap<string, Row>;
  /** Each adapter's Product and settings, from the first of its slots that started. */
  readonly ran: Map<string, Pick<AdapterRow, "product" | "settings">>;
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function submissionOf(
  task: string,
  answer: Exclude<RunAnswer, { status: "unsupported" }>,
): Submission {
  const submission = { task, status: answer.status, records: answer.records };
  return answer.error === undefined ? submission : { ...submission, error: answer.error };
}

/**
 * Runs `iterations` tasks with at most `concurrency` in flight, each started
 * as soon as one ends. A task due past `maxDurationMs` is rejected instead.
 * Returns the window's length.
 */
async function closedLoop(input: {
  readonly iterations: number;
  readonly concurrency: number;
  readonly maxDurationMs: number;
  readonly task: (index: number) => Promise<void>;
  readonly reject: (index: number) => Promise<void>;
}): Promise<number> {
  const start = performance.now();
  const pending = new Set<Promise<void>>();
  for (let index = 0; index < input.iterations; index += 1) {
    while (pending.size >= input.concurrency) {
      await Promise.race(pending);
    }
    if (performance.now() - start >= input.maxDurationMs) {
      await input.reject(index);
      continue;
    }
    const running: Promise<void> = input.task(index).finally(() => {
      pending.delete(running);
    });
    pending.add(running);
  }
  await Promise.all(pending);
  return performance.now() - start;
}

/** One attempt of `task` on the adapter `slot` runs: never throws, its failure is its status. */
async function attempt(input: {
  readonly context: RunContext;
  readonly slot: AdapterSlot;
  readonly shell: Pick<
    Attempt,
    "attempt_id" | "cell_id" | "adapter" | "task" | "block" | "phase" | "index"
  >;
  readonly task: ListedTask;
}): Promise<Attempt> {
  const { context, slot, shell, task } = input;
  const queued = performance.now();
  const ended = (
    fields: Partial<Omit<Attempt, keyof typeof shell | "status">> &
      Pick<Attempt, "status" | "timing">,
  ): Attempt => ({
    ...shell,
    error_code: null,
    reason: null,
    generation: Math.max(0, slot.generation),
    judgement: null,
    adapter_metrics: null,
    ...fields,
  });
  const failed = (error: unknown, timing: Attempt["timing"]): Attempt => {
    const code = errorCode(error);
    return ended({ status: statusFor(code), error_code: code, reason: messageOf(error), timing });
  };

  let client;
  try {
    client = await slot.ensure();
  } catch (error) {
    return failed(error, { queue_ms: performance.now() - queued, task_ms: null, oracle_ms: null });
  }
  const generation = slot.generation;
  const queueMs = performance.now() - queued;
  if (!slot.capabilities.includes(task.kind)) {
    return ended({
      status: "unsupported",
      error_code: "capability",
      reason: `the adapter does not claim the task kind ${task.kind}`,
      generation,
      timing: { queue_ms: queueMs, task_ms: null, oracle_ms: null },
    });
  }

  const sent = performance.now();
  let answer: RunAnswer;
  try {
    const result = await client.call("run", {
      task: {
        id: task.id,
        kind: task.kind,
        start_url: task.start_url,
        description: task.description,
      },
    });
    try {
      answer = runAnswer.parse(result, "adapter run answer");
    } catch (error) {
      throw new BenchError("invalid_output", messageOf(error), { cause: error });
    }
  } catch (error) {
    return {
      ...failed(error, { queue_ms: queueMs, task_ms: performance.now() - sent, oracle_ms: null }),
      generation,
    };
  }
  const taskMs = performance.now() - sent;
  if (answer.status === "unsupported") {
    return ended({
      status: "unsupported",
      error_code: "capability",
      reason: answer.reason,
      generation,
      timing: { queue_ms: queueMs, task_ms: taskMs, oracle_ms: null },
    });
  }
  const metrics = answer.metrics ?? null;
  let judgement: Judgement;
  try {
    judgement = await context.ground.judge(submissionOf(task.id, answer));
  } catch (error) {
    return {
      ...failed(error, { queue_ms: queueMs, task_ms: taskMs, oracle_ms: null }),
      generation,
      adapter_metrics: metrics,
    };
  }
  return ended({
    status: judgement.outcome,
    generation,
    judgement,
    adapter_metrics: metrics,
    timing: {
      queue_ms: queueMs,
      task_ms: taskMs,
      oracle_ms: judgement.oracle_time_microseconds / 1000,
    },
  });
}

/** Runs one cell: its gate, its warmup and, when both pass, its measured attempts. */
async function runCell(input: {
  readonly context: RunContext;
  readonly slot: AdapterSlot;
  readonly adapter: string;
  readonly task: ListedTask;
  readonly block: number;
}): Promise<CellRun> {
  const { context, slot, adapter, task, block } = input;
  const { campaign } = context;
  const cellId = `${adapter}:${task.id}:b${block}`;
  const attempts: Attempt[] = [];
  const shellOf = (phase: Phase, index: number): Parameters<typeof attempt>[0]["shell"] => ({
    attempt_id: `${cellId}:${phase}:${index}`,
    cell_id: cellId,
    adapter,
    task: task.id,
    block,
    phase,
    index,
  });
  const keep = async (one: Attempt): Promise<AttemptStatus> => {
    attempts.push(one);
    await context.record(one);
    return one.status;
  };
  const run = async (phase: Phase, index: number): Promise<AttemptStatus> =>
    await keep(await attempt({ context, slot, shell: shellOf(phase, index), task }));
  const skip = async (phase: Phase, index: number): Promise<void> => {
    await keep({
      ...shellOf(phase, index),
      status: "skipped_gate",
      error_code: null,
      reason: "the cell failed its conformance gate",
      timing: { queue_ms: null, task_ms: null, oracle_ms: null },
      generation: Math.max(0, slot.generation),
      judgement: null,
      adapter_metrics: null,
    });
  };

  let passed = (await run("conformance", 0)) === "verified";
  for (let index = 1; index <= campaign.warmup; index += 1) {
    if (passed) {
      passed = (await run("warmup", index)) === "verified";
    } else {
      await skip("warmup", index);
    }
  }
  const first = campaign.warmup + 1;
  let windowMs: number | null = null;
  if (passed) {
    windowMs = await closedLoop({
      iterations: campaign.iterations,
      concurrency: campaign.concurrency,
      maxDurationMs: campaign.max_duration_ms,
      task: async (index) => {
        await run("measure", first + index);
      },
      reject: async (index) => {
        await keep({
          ...shellOf("measure", first + index),
          status: "rejected",
          error_code: "deadline",
          reason: `due past the cell's ${campaign.max_duration_ms} ms window`,
          timing: { queue_ms: null, task_ms: null, oracle_ms: null },
          generation: Math.max(0, slot.generation),
          judgement: null,
          adapter_metrics: null,
        });
      },
    });
  } else {
    for (let index = 0; index < campaign.iterations; index += 1) {
      await skip("measure", first + index);
    }
  }
  return { cellId, adapter, task: task.id, block, passed, attempts, windowMs };
}

/** The row of the campaign's adapter `id`. */
function rowOf(context: RunContext, id: string): Row {
  const row = context.rows.get(id);
  if (row === undefined) {
    throw new Error(`invariant: the campaign gives its adapter ${id} a row`);
  }
  return row;
}

/** Runs one adapter's cells in one block, on one adapter slot sampled for its whole life. */
async function runAdapterBlock(input: {
  readonly context: RunContext;
  readonly adapter: CampaignAdapter;
  readonly tasks: readonly ListedTask[];
  readonly block: number;
  readonly pids: number[];
}): Promise<Cell[]> {
  const { context, adapter, block } = input;
  const { campaign } = context;
  const samplers: TreeSampler[] = [];
  const slot = new AdapterSlot({
    adapter: { command: adapter.command, cwd: context.folder, timeoutMs: campaign.timeout_ms },
    prepare: {
      proving_ground: { address: context.ground.address },
      concurrency: campaign.concurrency,
      row: rowOf(context, adapter.id),
    },
    maxRestarts: campaign.max_restarts,
    concurrency: campaign.concurrency,
    started: (pid) => {
      input.pids.push(pid);
      samplers.push(sampleProcessTree({ pid, probes: context.probes, intervalMs: 100 }));
    },
  });
  const runs: CellRun[] = [];
  let usages: TreeUsage[];
  try {
    for (const task of input.tasks) {
      runs.push(await runCell({ context, slot, adapter: adapter.id, task, block }));
    }
  } finally {
    // The samplers stop before the adapter, so their last sample still sees its tree.
    usages = await Promise.all(samplers.map(async (sampler) => await sampler.stop()));
    await slot.close();
    if (slot.generation >= 0 && !context.ran.has(adapter.id)) {
      context.ran.set(adapter.id, { product: slot.product, settings: slot.settings });
    }
  }
  const usage = mergeUsage(usages);
  const verifiedInBlock = runs
    .flatMap((run) => run.attempts)
    .filter((one) => one.status === "verified").length;
  return runs.map((run) =>
    summarize({ book: context.book, run, usage, verifiedInBlock, startupMs: slot.startupMs }),
  );
}

/** Where a run reads its campaign and Proving ground, and writes its folder. */
export interface RunInput {
  readonly campaignPath: string;
  /** The Proving ground's address, such as `127.0.0.1:51234`. */
  readonly provingGround: string;
  /** The folder the run's own folder is made in. */
  readonly out: string;
}

/** A finished run: its folder and its record. */
export interface RunResult {
  readonly directory: string;
  readonly record: RunRecord;
}

/**
 * Runs the campaign at `campaignPath` against the Proving ground and writes
 * `run.json` and `attempts.jsonl` in a new folder under `out`. A campaign task
 * the Proving ground does not list, or a refused host, fails before anything
 * runs or is written.
 */
export async function runCampaign(input: RunInput): Promise<RunResult> {
  const loaded = await loadCampaign(input.campaignPath);
  const { campaign } = loaded;
  const ground = provingGround(input.provingGround);
  const listed = await ground.tasks();
  const tasks = campaign.tasks.map((id) => {
    const task = listed.tasks.find((candidate) => candidate.id === id);
    if (task === undefined) {
      throw new BenchError("task_unknown", `the Proving ground lists no task ${id}`);
    }
    return task;
  });

  const provenance = await captureProvenance({ repository, sources });
  const preflight = await snapshotHost([process.pid]);
  const classes = loadHostClasses();
  const assessment = assessHost({
    snapshot: preflight,
    requested: campaign.host_class.requested,
    allowDowngrade: campaign.host_class.allow_downgrade,
    gitClean: provenance.git !== null && !provenance.git.dirty,
    classes,
  });
  if (assessment.refused) {
    const failed = assessment.gates.filter((gate) => !gate.ok).map((gate) => gate.id);
    throw new BenchError(
      "host_gate",
      `the host is refused for ${campaign.host_class.requested}: ${failed.join(", ")}`,
    );
  }
  const granted = assessment.class_granted;
  const gates: readonly Gate[] = classes.find((spec) => spec.class === granted)?.gates ?? [];

  const startedAt = new Date().toISOString();
  const runId = `${campaign.id}-${startedAt.replaceAll(/[:.]/g, "-")}-${randomUUID().slice(0, 8)}`;
  const directory = join(input.out, runId);
  await mkdir(directory, { recursive: true });
  const journal = join(directory, "attempts.jsonl");
  const attempts: Attempt[] = [];
  let writes = Promise.resolve();
  const probes = detectProbes(platform());
  const context: RunContext = {
    campaign,
    folder: loaded.folder,
    ground,
    probes,
    book: metricBook({ platform: platform(), probes }),
    record: async (one) => {
      attempts.push(one);
      writes = writes.then(async () => await appendFile(journal, `${JSON.stringify(one)}\n`));
      await writes;
    },
    rows: loaded.rows,
    ran: new Map(),
  };

  const blocks: { block: number; order: string[]; noisy: boolean }[] = [];
  const cells: Cell[] = [];
  for (let block = 0; block < campaign.blocks; block += 1) {
    const order = shuffle(
      campaign.adapters.map((adapter) => adapter.id),
      campaign.order_seed + block,
    );
    const pids: number[] = [];
    for (const id of order) {
      const adapter = campaign.adapters.find((candidate) => candidate.id === id);
      if (adapter === undefined) {
        throw new Error(`invariant: the shuffled adapter ${id} is the campaign's`);
      }
      cells.push(...(await runAdapterBlock({ context, adapter, tasks, block, pids })));
    }
    // `local-dev` has no gates, so it never reads the host for a block.
    const noisy =
      gates.length > 0 &&
      blockNoisy({
        gates,
        foreignCpuPercent: (await snapshotHost([process.pid, ...pids])).foreign_cpu_percent,
      });
    blocks.push({ block, order, noisy });
  }
  await writes;

  const changed = changedSources(provenance, await captureProvenance({ repository, sources }));
  const postflight = await snapshotHost([process.pid]);
  const status: RunStatus =
    changed.length > 0
      ? "invalid_provenance"
      : blocks.some((one) => one.noisy)
        ? "noisy_host"
        : "complete";
  const record: RunRecord = {
    schema_version: 1,
    run_id: runId,
    started_at: startedAt,
    finished_at: new Date().toISOString(),
    status,
    campaign: { id: campaign.id, sha256: loaded.sha256, document: campaign },
    provenance,
    changed_sources: changed,
    proving_ground: {
      version: listed.proving_ground.version,
      seed: listed.proving_ground.seed,
      tasks: tasks.map((task) => ({ id: task.id, digest: task.digest })),
    },
    host: {
      class_requested: campaign.host_class.requested,
      class_granted: granted,
      preflight: { snapshot: preflight, gates: assessment.gates },
      postflight: { snapshot: postflight },
    },
    metric_availability: context.book.availability,
    blocks,
    adapters: campaign.adapters.map((adapter) => ({
      id: adapter.id,
      product: context.ran.get(adapter.id)?.product ?? null,
      row: rowOf(context, adapter.id),
      settings: context.ran.get(adapter.id)?.settings ?? null,
    })),
    cells,
    totals: countStatuses(attempts),
  };
  await writeFile(join(directory, "run.json"), `${JSON.stringify(record, null, 2)}\n`);
  return { directory, record };
}
