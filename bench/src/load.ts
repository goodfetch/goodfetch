// Load runs and concurrency search (ADRs 0025 and 0040):
// runs a family across declared concurrency levels (defaulting to 1, 8, 64
// and 256) and searches for the highest concurrency where verified throughput
// still rises and false success stays at zero. Publishes the resulting
// governor default ceilings in a schema-validated JSON document that the
// Engine runtime area reads (ADR 0040).
import type { Product } from "./evaluations.ts";
import { hostClasses, type HostClass } from "./host.ts";
import { parserOf } from "./schema.ts";

/** Standard concurrency levels specified by ADR 0025. */
export const STANDARD_CONCURRENCY_LEVELS = [1, 8, 64, 256] as const;

/** Measurement observed at one concurrency level. */
export interface LoadLevelMeasurement {
  readonly concurrency: number;
  readonly verified_tasks_per_second: number;
  readonly verified: number;
  readonly false_success: number;
  readonly failed: number;
  readonly total_attempts: number;
}

/** Why the concurrency search stopped. */
export type ConcurrencySearchStopReason =
  | "false_success"
  | "throughput_plateau"
  | "exhausted_levels";

/** Outcome of searching for the highest safe concurrency level. */
export interface ConcurrencySearchResult {
  /** Highest concurrency where verified throughput rose and false_success === 0, or `null` if level 1 failed. */
  readonly highest_safe_concurrency: number | null;
  readonly peak_verified_tasks_per_second: number | null;
  readonly stop_reason: ConcurrencySearchStopReason;
  /** The concurrency level at which the search stopped (or the last tested level if exhausted). */
  readonly stopped_at_concurrency: number;
  /** Every concurrency level actually measured before stopping, in order. */
  readonly levels: readonly LoadLevelMeasurement[];
}

/** Work kinds governed by the Engine runtime Resource governor (ADR 0040, `goodfetch-protocol::governor::WorkKind`). */
export const GOVERNOR_WORK_KINDS = ["http", "browser_page", "model_call"] as const;
export type GovernorWorkKind = (typeof GOVERNOR_WORK_KINDS)[number];

/** Engine runtime ceiling setting key for each `WorkKind`. */
export type GovernorCeilingSetting =
  | "governor.http.ceiling"
  | "governor.browser_page.ceiling"
  | "governor.model_call.ceiling";

/** Maps a work kind to its exact Engine runtime setting key. */
export function settingForWorkKind(kind: GovernorWorkKind): GovernorCeilingSetting {
  switch (kind) {
    case "http":
      return "governor.http.ceiling";
    case "browser_page":
      return "governor.browser_page.ceiling";
    case "model_call":
      return "governor.model_call.ceiling";
    default: {
      const exhaustive: never = kind;
      return exhaustive;
    }
  }
}

/** Maps a benchmark family name to the primary governor `WorkKind` it exercises. */
export function workKindForFamily(family: string): GovernorWorkKind {
  switch (family) {
    case "http-crawl":
    case "run-state-and-admission":
    case "incident-recovery":
      return "http";
    case "ai":
      return "model_call";
    default:
      return "browser_page";
  }
}

/** Published default ceilings per Engine runtime `WorkKind` (ADR 0040). */
export interface GovernorWorkKindCeilings {
  readonly http: number | null;
  readonly browser_page: number | null;
  readonly model_call: number | null;
}

/** Published default ceilings document read by the Engine runtime governor (ADR 0040). */
export interface GovernorCeilings {
  readonly schema_version: 1;
  readonly product: Product;
  readonly family: string;
  readonly work_kind: GovernorWorkKind;
  readonly setting: GovernorCeilingSetting;
  readonly default_ceiling: number;
  readonly ceilings: GovernorWorkKindCeilings;
  readonly peak_verified_tasks_per_second: number;
  readonly stop_reason: ConcurrencySearchStopReason;
  readonly stopped_at_concurrency: number;
  readonly host_class: HostClass;
  readonly measured_at: string;
  readonly provenance_commit: string;
  readonly levels: readonly LoadLevelMeasurement[];
}

const nullableCeiling = { anyOf: [{ type: "integer", minimum: 1 }, { type: "null" }] };

const governorCeilingsParser = parserOf<GovernorCeilings>({
  type: "object",
  additionalProperties: false,
  required: [
    "schema_version",
    "product",
    "family",
    "work_kind",
    "setting",
    "default_ceiling",
    "ceilings",
    "peak_verified_tasks_per_second",
    "stop_reason",
    "stopped_at_concurrency",
    "host_class",
    "measured_at",
    "provenance_commit",
    "levels",
  ],
  properties: {
    schema_version: { const: 1 },
    product: { enum: ["collie", "foxhound"] },
    family: { type: "string", minLength: 1 },
    work_kind: { enum: [...GOVERNOR_WORK_KINDS] },
    setting: {
      enum: [
        "governor.http.ceiling",
        "governor.browser_page.ceiling",
        "governor.model_call.ceiling",
      ],
    },
    default_ceiling: { type: "integer", minimum: 1 },
    ceilings: {
      type: "object",
      additionalProperties: false,
      required: ["http", "browser_page", "model_call"],
      properties: {
        http: nullableCeiling,
        browser_page: nullableCeiling,
        model_call: nullableCeiling,
      },
    },
    peak_verified_tasks_per_second: { type: "number", minimum: 0 },
    stop_reason: { enum: ["false_success", "throughput_plateau", "exhausted_levels"] },
    stopped_at_concurrency: { type: "integer", minimum: 1 },
    host_class: { enum: [...hostClasses] },
    measured_at: {
      type: "string",
      pattern: "^\\d{4}-\\d{2}-\\d{2}T\\d{2}:\\d{2}:\\d{2}(\\.\\d+)?Z$",
    },
    provenance_commit: { type: "string", minLength: 1 },
    levels: {
      type: "array",
      minItems: 1,
      items: {
        type: "object",
        additionalProperties: false,
        required: [
          "concurrency",
          "verified_tasks_per_second",
          "verified",
          "false_success",
          "failed",
          "total_attempts",
        ],
        properties: {
          concurrency: { type: "integer", minimum: 1 },
          verified_tasks_per_second: { type: "number", minimum: 0 },
          verified: { type: "integer", minimum: 0 },
          false_success: { type: "integer", minimum: 0 },
          failed: { type: "integer", minimum: 0 },
          total_attempts: { type: "integer", minimum: 0 },
        },
      },
    },
  },
});

/** Validates a governor ceilings document at the boundary. */
export function parseGovernorCeilings(raw: unknown, label = "governor ceilings"): GovernorCeilings {
  return governorCeilingsParser.parse(raw, label);
}

/**
 * Searches `levels` in ascending order and stops at the first level where
 * verified throughput stops strictly rising or a false success appears.
 */
export async function runConcurrencySearch(input: {
  readonly levels?: readonly number[] | undefined;
  readonly measure: (concurrency: number) => Promise<LoadLevelMeasurement>;
}): Promise<ConcurrencySearchResult> {
  const ordered = [...new Set(input.levels ?? STANDARD_CONCURRENCY_LEVELS)].toSorted(
    (a, b) => a - b,
  );
  if (ordered.length === 0) {
    throw new Error("concurrency search requires at least one concurrency level");
  }

  const measured: LoadLevelMeasurement[] = [];
  let bestConcurrency: number | null = null;
  let bestThroughput: number | null = null;

  for (const level of ordered) {
    const sample = await input.measure(level);
    measured.push(sample);

    if (sample.false_success > 0) {
      return {
        highest_safe_concurrency: bestConcurrency,
        peak_verified_tasks_per_second: bestThroughput,
        stop_reason: "false_success",
        stopped_at_concurrency: level,
        levels: measured,
      };
    }

    if (bestThroughput !== null && sample.verified_tasks_per_second <= bestThroughput) {
      return {
        highest_safe_concurrency: bestConcurrency,
        peak_verified_tasks_per_second: bestThroughput,
        stop_reason: "throughput_plateau",
        stopped_at_concurrency: level,
        levels: measured,
      };
    }

    bestConcurrency = level;
    bestThroughput = sample.verified_tasks_per_second;
  }

  return {
    highest_safe_concurrency: bestConcurrency,
    peak_verified_tasks_per_second: bestThroughput,
    stop_reason: "exhausted_levels",
    stopped_at_concurrency: ordered.at(-1) ?? 1,
    levels: measured,
  };
}

/** Formats a successful concurrency search as the Engine runtime's governor ceilings document. */
export function toGovernorCeilings(input: {
  readonly product: Product;
  readonly family: string;
  readonly workKind?: GovernorWorkKind | undefined;
  readonly search: ConcurrencySearchResult;
  readonly hostClass: HostClass;
  readonly measuredAt: string;
  readonly provenanceCommit: string;
}): GovernorCeilings {
  if (
    input.search.highest_safe_concurrency === null ||
    input.search.peak_verified_tasks_per_second === null
  ) {
    throw new Error("cannot publish governor ceilings when no concurrency level was safe");
  }
  const workKind = input.workKind ?? workKindForFamily(input.family);
  const ceiling = input.search.highest_safe_concurrency;
  const doc: GovernorCeilings = {
    schema_version: 1,
    product: input.product,
    family: input.family,
    work_kind: workKind,
    setting: settingForWorkKind(workKind),
    default_ceiling: ceiling,
    ceilings: {
      http: workKind === "http" ? ceiling : null,
      browser_page: workKind === "browser_page" ? ceiling : null,
      model_call: workKind === "model_call" ? ceiling : null,
    },
    peak_verified_tasks_per_second: input.search.peak_verified_tasks_per_second,
    stop_reason: input.search.stop_reason,
    stopped_at_concurrency: input.search.stopped_at_concurrency,
    host_class: input.hostClass,
    measured_at: input.measuredAt,
    provenance_commit: input.provenanceCommit,
    levels: input.search.levels,
  };
  return parseGovernorCeilings(doc, "governor ceilings");
}
