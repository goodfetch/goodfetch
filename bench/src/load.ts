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

/** Published default ceilings document read by the Engine runtime governor (ADR 0040). */
export interface GovernorCeilings {
  readonly schema_version: 1;
  readonly product: Product;
  readonly family: string;
  readonly default_ceiling: number;
  readonly peak_verified_tasks_per_second: number;
  readonly stop_reason: ConcurrencySearchStopReason;
  readonly stopped_at_concurrency: number;
  readonly host_class: HostClass;
  readonly measured_at: string;
  readonly provenance_commit: string;
  readonly levels: readonly LoadLevelMeasurement[];
}

const governorCeilingsParser = parserOf<GovernorCeilings>({
  type: "object",
  additionalProperties: false,
  required: [
    "schema_version",
    "product",
    "family",
    "default_ceiling",
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
    default_ceiling: { type: "integer", minimum: 1 },
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
  readonly levels?: readonly number[];
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
  const doc: GovernorCeilings = {
    schema_version: 1,
    product: input.product,
    family: input.family,
    default_ceiling: input.search.highest_safe_concurrency,
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
