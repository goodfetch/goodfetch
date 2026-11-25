// Decisions and claims (ADR 0025), declared before anything is measured and
// carried over from the lab's registry. A decision names its options (each
// an adapter of the campaigns that measure it), its baseline, its primary
// metric, its contract gates, the smallest effect Δ that matters and its
// fixed rule; a claim states one comparison on one of its metrics, with its
// direction and Δ. Both are dated: a run records what it declared when it
// started, and a claim or decision that cites a run it was not declared
// before is refused. Editorial text never carries a measured figure.
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import { hostClasses, type HostClass } from "./host.ts";
import { metricCatalog, type MetricId } from "./metrics.ts";
import { parserOf } from "./schema.ts";

/** A gate a decision's options must pass before any effect counts. */
export type ContractGate = "oracle_exact" | "false_success_count";

/** A decision's fixed rule; noisy blocks are left out of its estimate, or make it inconclusive. */
export interface DecisionRule {
  readonly kind: "superiority" | "non-inferiority" | "equivalence";
  readonly alpha: number;
  readonly power: number;
  readonly noisy_blocks: "exclude" | "inconclusive";
}

/** A question with options, answered by verdicts on the runs it cites. */
export interface Decision {
  readonly id: string;
  readonly title: string;
  readonly question: string;
  readonly status: "open" | "evidence-pending" | "decided" | "superseded" | "deferred";
  /** When it was declared, an ISO instant; every run it cites started after it. */
  readonly declared_at: string;
  /** Each option is one adapter of the campaigns that measure it. */
  readonly options: readonly {
    readonly id: string;
    readonly description: string;
    readonly adapter: string;
  }[];
  readonly baseline: string;
  readonly primary_metric: MetricId;
  readonly secondary_metrics: readonly MetricId[];
  readonly contract_gates: readonly ContractGate[];
  /** The smallest effect of interest, as a ratio: 0.1 is 10%. */
  readonly delta: number;
  readonly rule: DecisionRule;
  /** The host class a verdict needs, and the one a published figure needs. */
  readonly host_class: { readonly decision: HostClass; readonly publication: HostClass };
  /** Sets of tasks judged apart, so opposite effects are not pooled; all tasks when absent. */
  readonly strata?: readonly { readonly id: string; readonly tasks: readonly string[] }[];
  /** The runs its verdicts read, by ID. */
  readonly evidence: readonly string[];
  readonly depends_on: readonly string[];
  readonly notes: string;
}

/** One comparison a decision's verdict supports or refutes. */
export interface Claim {
  readonly id: string;
  readonly statement: string;
  readonly decision: string;
  readonly metric: MetricId;
  readonly comparator: { readonly option: string; readonly against: readonly string[] };
  readonly direction: "lower" | "higher" | "zero";
  /** The smallest effect it claims, as a ratio. */
  readonly delta: number;
  readonly publication_host_class: "local-controlled" | "reference";
  readonly status: "candidate" | "supported" | "refuted" | "withdrawn";
  readonly declared_at: string;
  readonly evidence: readonly string[];
  readonly notes: string;
}

/** Every decision and claim of one registry folder. */
export interface Registry {
  readonly decisions: readonly Decision[];
  readonly claims: readonly Claim[];
}

/** What a run declared when it started: each decision and claim with the date it then had. */
export interface Declared {
  readonly decisions: readonly { readonly id: string; readonly declared_at: string }[];
  readonly claims: readonly { readonly id: string; readonly declared_at: string }[];
}

/** The part of a run record the registry's checks read. */
export interface DeclaredRun {
  readonly run_id: string;
  readonly started_at: string;
  readonly declared: Declared;
}

const text = { type: "string" };
const instant = {
  type: "string",
  pattern: "^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}(\\.\\d+)?Z$",
};
const metric = { enum: Object.keys(metricCatalog) };
const ids = { type: "array", items: text };
const ratio = { type: "number", exclusiveMinimum: 0 };

const decisionFile = parserOf<Decision>({
  type: "object",
  additionalProperties: false,
  required: [
    "id",
    "title",
    "question",
    "status",
    "declared_at",
    "options",
    "baseline",
    "primary_metric",
    "secondary_metrics",
    "contract_gates",
    "delta",
    "rule",
    "host_class",
    "evidence",
    "depends_on",
    "notes",
  ],
  properties: {
    id: { type: "string", pattern: "^D-[a-z0-9-]+$" },
    title: { type: "string", minLength: 1 },
    question: { type: "string", minLength: 1 },
    status: { enum: ["open", "evidence-pending", "decided", "superseded", "deferred"] },
    declared_at: instant,
    options: {
      type: "array",
      items: {
        type: "object",
        additionalProperties: false,
        required: ["id", "description", "adapter"],
        properties: { id: text, description: text, adapter: text },
      },
    },
    baseline: text,
    primary_metric: metric,
    secondary_metrics: { type: "array", items: metric },
    contract_gates: { type: "array", items: { enum: ["oracle_exact", "false_success_count"] } },
    delta: ratio,
    rule: {
      type: "object",
      additionalProperties: false,
      required: ["kind", "alpha", "power", "noisy_blocks"],
      properties: {
        kind: { enum: ["superiority", "non-inferiority", "equivalence"] },
        alpha: { type: "number", exclusiveMinimum: 0, maximum: 0.2 },
        power: { type: "number", minimum: 0.5, exclusiveMaximum: 1 },
        noisy_blocks: { enum: ["exclude", "inconclusive"] },
      },
    },
    host_class: {
      type: "object",
      additionalProperties: false,
      required: ["decision", "publication"],
      properties: { decision: { enum: [...hostClasses] }, publication: { enum: [...hostClasses] } },
    },
    strata: {
      type: "array",
      minItems: 1,
      items: {
        type: "object",
        additionalProperties: false,
        required: ["id", "tasks"],
        properties: { id: text, tasks: { type: "array", minItems: 1, items: text } },
      },
    },
    evidence: ids,
    depends_on: ids,
    notes: text,
  },
});

const claimFile = parserOf<Claim>({
  type: "object",
  additionalProperties: false,
  required: [
    "id",
    "statement",
    "decision",
    "metric",
    "comparator",
    "direction",
    "delta",
    "publication_host_class",
    "status",
    "declared_at",
    "evidence",
    "notes",
  ],
  properties: {
    id: { type: "string", pattern: "^C-[a-z0-9-]+$" },
    statement: { type: "string", minLength: 1 },
    decision: { type: "string", pattern: "^D-[a-z0-9-]+$" },
    metric,
    comparator: {
      type: "object",
      additionalProperties: false,
      required: ["option", "against"],
      properties: { option: text, against: { type: "array", minItems: 1, items: text } },
    },
    direction: { enum: ["lower", "higher", "zero"] },
    delta: ratio,
    publication_host_class: { enum: ["local-controlled", "reference"] },
    status: { enum: ["candidate", "supported", "refuted", "withdrawn"] },
    declared_at: instant,
    evidence: ids,
    notes: text,
  },
});

async function documents<T>(
  folder: string,
  parse: (value: unknown, what: string) => T,
): Promise<T[]> {
  const names = await readdir(folder).catch((error: unknown) => {
    if (error instanceof Error && "code" in error && error.code === "ENOENT") {
      return [];
    }
    throw error;
  });
  const files = names.filter((name) => name.endsWith(".json")).toSorted();
  return await Promise.all(
    files.map(async (name) => {
      const path = join(folder, name);
      return parse(JSON.parse(await readFile(path, "utf8")), path);
    }),
  );
}

/** Reads `decisions/*.json` and `claims/*.json` under `root`, each against its schema; a missing folder holds none. */
export async function loadRegistry(root: string): Promise<Registry> {
  return {
    decisions: await documents(join(root, "decisions"), (value, what) =>
      decisionFile.parse(value, what),
    ),
    claims: await documents(join(root, "claims"), (value, what) => claimFile.parse(value, what)),
  };
}

/** A measured figure: these belong to generated verdicts, never to editorial text. */
const measuredFigure =
  /\d+(?:[.,]\d+)?\s*(?:%|ms\b|s\b|µs\b|[KMG]i?B\b|[KMG]o\b|B\b|jobs?\/s|(?:req|requests)\/s|rows?\/s|tasks?\/s|×|x\b|fois\b)/i;

/** The first measured figure in editorial `text`, or `null`. */
function figureIn(...texts: readonly string[]): string | null {
  for (const one of texts) {
    const found = measuredFigure.exec(one);
    if (found !== null) {
      return found[0];
    }
  }
  return null;
}

function duplicates(values: readonly string[]): string[] {
  return values.filter((value, index) => values.indexOf(value) !== index);
}

/** What the checks compare declarations with: the time now, and the runs evidence may cite. */
export interface CheckContext {
  /** Before a run, its start: nothing it measures may be declared later. */
  readonly now?: Date;
  /** When given, every evidence run must be one of these and have declared what cites it. */
  readonly runs?: readonly DeclaredRun[];
}

/**
 * Every problem of `registry`, as `<id>: <problem>` lines: unknown references,
 * a figure in editorial text, a declaration after `now`, and evidence from a
 * run that did not declare it before measuring.
 */
export function checkRegistry(registry: Registry, context: CheckContext = {}): string[] {
  const errors: string[] = [];
  const decisions = new Map(registry.decisions.map((one) => [one.id, one]));
  const runs =
    context.runs === undefined ? null : new Map(context.runs.map((one) => [one.run_id, one]));

  const dated = (
    id: string,
    declaredAt: string,
    evidence: readonly string[],
    recorded: (
      run: DeclaredRun,
    ) => readonly { readonly id: string; readonly declared_at: string }[],
  ): void => {
    const at = (message: string) => errors.push(`${id}: ${message}`);
    const declared = Date.parse(declaredAt);
    if (Number.isNaN(declared)) {
      at(`declared at ${declaredAt}, which is no instant`);
      return;
    }
    if (context.now !== undefined && declared > context.now.getTime()) {
      at(`declared at ${declaredAt}, after now`);
    }
    if (runs === null) {
      return;
    }
    for (const runId of evidence) {
      const run = runs.get(runId);
      if (run === undefined) {
        at(`evidence run ${runId} is not a known run`);
        continue;
      }
      const record = recorded(run).find((one) => one.id === id);
      if (record === undefined) {
        at(`run ${runId} did not declare it before measuring`);
        continue;
      }
      if (declared > Date.parse(run.started_at)) {
        at(`declared at ${declaredAt}, after its run ${runId} started at ${run.started_at}`);
      }
      if (record.declared_at !== declaredAt) {
        at(`declared at ${declaredAt}, but run ${runId} recorded ${record.declared_at}`);
      }
    }
  };

  for (const id of duplicates(registry.decisions.map((one) => one.id))) {
    errors.push(`${id}: duplicate decision id`);
  }
  for (const decision of registry.decisions) {
    const at = (message: string) => errors.push(`${decision.id}: ${message}`);
    const options = decision.options.map((option) => option.id);
    if (options.length < 2) {
      at("a decision compares at least two options");
    }
    for (const id of duplicates(options)) {
      at(`duplicate option ${id}`);
    }
    if (!options.includes(decision.baseline)) {
      at(`baseline ${decision.baseline} is not one of its options`);
    }
    for (const dependency of decision.depends_on) {
      if (!decisions.has(dependency)) {
        at(`depends on unknown decision ${dependency}`);
      }
    }
    const figure = figureIn(decision.notes);
    if (figure !== null) {
      at(`notes contain a measured figure (${figure}); figures must come from verdicts`);
    }
    dated(decision.id, decision.declared_at, decision.evidence, (run) => run.declared.decisions);
  }

  for (const id of duplicates(registry.claims.map((one) => one.id))) {
    errors.push(`${id}: duplicate claim id`);
  }
  for (const claim of registry.claims) {
    const at = (message: string) => errors.push(`${claim.id}: ${message}`);
    const decision = decisions.get(claim.decision);
    if (decision === undefined) {
      at(`refers to unknown decision ${claim.decision}`);
      continue;
    }
    const options = new Set(decision.options.map((option) => option.id));
    for (const option of [claim.comparator.option, ...claim.comparator.against]) {
      if (!options.has(option)) {
        at(`option ${option} is not an option of ${decision.id}`);
      }
    }
    if (![decision.primary_metric, ...decision.secondary_metrics].includes(claim.metric)) {
      at(`metric ${claim.metric} is not measured by ${decision.id}`);
    } else if (claim.metric === decision.primary_metric && claim.delta !== decision.delta) {
      at(
        `its Δ ${claim.delta} is not ${decision.id}'s ${decision.delta} on the decision's primary metric`,
      );
    }
    const figure = figureIn(claim.statement, claim.notes);
    if (figure !== null) {
      at(
        `statement or notes contain a measured figure (${figure}); figures must come from verdicts`,
      );
    }
    dated(claim.id, claim.declared_at, claim.evidence, (run) => run.declared.claims);
  }
  return errors;
}
