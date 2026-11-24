// A campaign file: the competitor adapters to run, the Proving ground tasks
// they run, and how often, in how many blocks and at what concurrency. Each
// adapter starts in the campaign file's folder.
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { hostClasses, type HostClass } from "./host.ts";
import { parserOf } from "./schema.ts";

/** One competitor adapter of a campaign. */
export interface CampaignAdapter {
  readonly id: string;
  /** The program and its arguments, run in the campaign file's folder. */
  readonly command: readonly [string, ...string[]];
}

/** A campaign, as its file declares it. */
export interface Campaign {
  readonly id: string;
  readonly host_class: { readonly requested: HostClass; readonly allow_downgrade: boolean };
  /** The seed of the adapters' order in each block. */
  readonly order_seed: number;
  /** Measured blocks; the adapters' order is drawn again for each. */
  readonly blocks: number;
  /** Attempts of each cell after its conformance attempt and before it is measured. */
  readonly warmup: number;
  /** Measured attempts of each cell in each block. */
  readonly iterations: number;
  /** Measured attempts in flight at once on one adapter. */
  readonly concurrency: number;
  /** How long one call to an adapter may take. */
  readonly timeout_ms: number;
  /** How long a cell's measured window may last; later attempts are rejected. */
  readonly max_duration_ms: number;
  /** How often an adapter may be restarted in one block. */
  readonly max_restarts: number;
  readonly adapters: readonly CampaignAdapter[];
  /** The Proving ground tasks each adapter runs, by ID. */
  readonly tasks: readonly string[];
}

const count = { type: "integer", minimum: 0 };
const positive = { type: "integer", minimum: 1 };

const campaignFile = parserOf<Campaign>({
  type: "object",
  additionalProperties: false,
  required: [
    "id",
    "host_class",
    "order_seed",
    "blocks",
    "warmup",
    "iterations",
    "concurrency",
    "timeout_ms",
    "max_duration_ms",
    "max_restarts",
    "adapters",
    "tasks",
  ],
  properties: {
    id: { type: "string", pattern: "^[a-z0-9][a-z0-9-]*$" },
    host_class: {
      type: "object",
      additionalProperties: false,
      required: ["requested", "allow_downgrade"],
      properties: {
        requested: { enum: [...hostClasses] },
        allow_downgrade: { type: "boolean" },
      },
    },
    order_seed: count,
    blocks: positive,
    warmup: count,
    iterations: positive,
    concurrency: positive,
    timeout_ms: positive,
    max_duration_ms: positive,
    max_restarts: count,
    adapters: {
      type: "array",
      minItems: 1,
      items: {
        type: "object",
        additionalProperties: false,
        required: ["id", "command"],
        properties: {
          id: { type: "string", pattern: "^[a-z0-9][a-z0-9-]*$" },
          command: { type: "array", minItems: 1, items: { type: "string" } },
        },
      },
    },
    tasks: { type: "array", minItems: 1, items: { type: "string" } },
  },
});

/** A campaign read from its file. */
export interface LoadedCampaign {
  readonly campaign: Campaign;
  readonly path: string;
  /** The folder its adapters start in. */
  readonly folder: string;
  /** The SHA-256 of the file's bytes. */
  readonly sha256: string;
}

/** Reads and checks the campaign file at `path`. */
export async function loadCampaign(path: string): Promise<LoadedCampaign> {
  const absolute = resolve(path);
  const bytes = await readFile(absolute);
  const campaign = campaignFile.parse(JSON.parse(bytes.toString("utf8")), absolute);
  return {
    campaign,
    path: absolute,
    folder: dirname(absolute),
    sha256: createHash("sha256").update(bytes).digest("hex"),
  };
}
