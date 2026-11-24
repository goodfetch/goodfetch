// The results ledger, carried over from the lab: the mechanical facts of each
// run beside a person's classification of it. A selected run is cited and
// has a validity class, a superseded one names its existing replacement, and
// every decision named exists; a run nobody has classified yet enters as
// `unreviewed` with the decisions it declared, and a classification that
// calls a complete run's host noisy makes it `noisy_host`.
import { createHash } from "node:crypto";
import type { HostClass } from "./host.ts";
import type { Declared } from "./registry.ts";
import type { RunStatus } from "./runner.ts";
import { parserOf } from "./schema.ts";

/** A person's judgement of one run. */
export interface ClassificationEntry {
  readonly role: "selected" | "superseded" | "diagnostic" | "smoke" | "orchestrator";
  readonly reason: string;
  readonly superseded_by: string | null;
  readonly decisions: readonly string[];
  readonly validity: readonly {
    readonly scope: string;
    readonly class: "A" | "B" | "C" | "none";
    readonly note: string;
  }[];
  readonly noisy_host: boolean;
  readonly rerun: {
    readonly required: boolean;
    readonly priority: "P0" | "P1" | "P2" | null;
    readonly host_class: "local-controlled" | "reference" | null;
  };
}

/** Every classified run, by ID. */
export interface Classification {
  readonly schema_version: 1;
  readonly runs: Readonly<Record<string, ClassificationEntry>>;
}

const classificationFile = parserOf<Classification>({
  type: "object",
  additionalProperties: false,
  required: ["schema_version", "runs"],
  properties: {
    schema_version: { const: 1 },
    runs: {
      type: "object",
      additionalProperties: {
        type: "object",
        additionalProperties: false,
        required: [
          "role",
          "reason",
          "superseded_by",
          "decisions",
          "validity",
          "noisy_host",
          "rerun",
        ],
        properties: {
          role: { enum: ["selected", "superseded", "diagnostic", "smoke", "orchestrator"] },
          reason: { type: "string", minLength: 1 },
          superseded_by: { type: ["string", "null"] },
          decisions: { type: "array", items: { type: "string", pattern: "^D-[a-z0-9-]+$" } },
          validity: {
            type: "array",
            items: {
              type: "object",
              additionalProperties: false,
              required: ["scope", "class", "note"],
              properties: {
                scope: { type: "string" },
                class: { enum: ["A", "B", "C", "none"] },
                note: { type: "string" },
              },
            },
          },
          noisy_host: { type: "boolean" },
          rerun: {
            type: "object",
            additionalProperties: false,
            required: ["required", "priority", "host_class"],
            properties: {
              required: { type: "boolean" },
              priority: { enum: [null, "P0", "P1", "P2"] },
              host_class: { enum: [null, "local-controlled", "reference"] },
            },
          },
        },
      },
    },
  },
});

/** Reads a classification document, or throws `invalid_document` naming `what`. */
export function parseClassification(value: unknown, what: string): Classification {
  return classificationFile.parse(value, what);
}

/** The part of a run record the ledger reads. */
export interface LedgerRun {
  readonly run_id: string;
  readonly started_at: string;
  readonly status: RunStatus;
  readonly campaign: { readonly id: string };
  readonly host: { readonly class_granted: HostClass };
  readonly declared: Declared;
}

/** One run in the ledger. */
export interface LedgerEntry {
  readonly run_id: string;
  readonly campaign: string;
  readonly started_at: string;
  readonly status: RunStatus;
  readonly host_class: HostClass;
  readonly role: ClassificationEntry["role"] | "unreviewed";
  readonly superseded_by: string | null;
  readonly reason: string;
  readonly validity: ClassificationEntry["validity"];
  readonly decisions: readonly string[];
  /** The reports that cite it, sorted, each once. */
  readonly cited_in: readonly string[];
  readonly rerun: ClassificationEntry["rerun"];
}

/** The ledger, and the SHA-256 of the classification it was built from. */
export interface Ledger {
  readonly schema_version: 1;
  readonly classification_sha256: string;
  readonly entries: readonly LedgerEntry[];
}

/**
 * The ledger of `runs` under `classification`, with every problem as a
 * `<run>: <problem>` line. `citations` maps a run ID to the reports that
 * cite it; `decisions` are the registry's decision IDs.
 */
export function buildLedger(input: {
  readonly runs: readonly LedgerRun[];
  readonly classification: Classification;
  readonly citations: Readonly<Record<string, readonly string[]>>;
  readonly decisions: readonly string[];
}): { errors: string[]; ledger: Ledger } {
  const { runs, classification, citations, decisions } = input;
  const errors: string[] = [];
  const runIds = new Set(runs.map((run) => run.run_id));
  for (const id of Object.keys(classification.runs)) {
    if (!runIds.has(id)) {
      errors.push(`${id}: classified but not a result run`);
    }
  }
  const entries = runs.map((run): LedgerEntry => {
    const at = (message: string) => errors.push(`${run.run_id}: ${message}`);
    const classified = classification.runs[run.run_id];
    const entry = classified ?? {
      role: "unreviewed" as const,
      reason: "awaiting classification",
      superseded_by: null,
      decisions: run.declared.decisions.map((decision) => decision.id),
      validity: [],
      noisy_host: false,
      rerun: { required: false, priority: null, host_class: null },
    };
    const cited = [...new Set(citations[run.run_id] ?? [])].toSorted();
    if ((entry.role === "selected" || entry.role === "orchestrator") && cited.length === 0) {
      at(`role ${entry.role} but not cited in any report`);
    }
    if (
      entry.role === "superseded" &&
      (entry.superseded_by === null || !runIds.has(entry.superseded_by))
    ) {
      at(`superseded by ${entry.superseded_by ?? "nothing"}, which is not a result run`);
    }
    for (const decision of entry.decisions) {
      if (!decisions.includes(decision)) {
        at(`unknown decision ${decision}`);
      }
    }
    if (entry.role === "selected" && entry.validity.length === 0) {
      at("selected without a validity class");
    }
    return {
      run_id: run.run_id,
      campaign: run.campaign.id,
      started_at: run.started_at,
      status: entry.noisy_host && run.status === "complete" ? "noisy_host" : run.status,
      host_class: run.host.class_granted,
      role: entry.role,
      superseded_by: entry.superseded_by,
      reason: entry.reason,
      validity: entry.validity,
      decisions: [...entry.decisions],
      cited_in: cited,
      rerun: entry.rerun,
    };
  });
  return {
    errors,
    ledger: {
      schema_version: 1,
      classification_sha256: createHash("sha256")
        .update(JSON.stringify(classification))
        .digest("hex"),
      entries,
    },
  };
}
