// A campaign file: the competitor adapters to run, the Proving ground tasks
// they run, and how often, in how many blocks and at what concurrency. Each
// adapter starts in the campaign file's folder.
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { BenchError } from "./errors.ts";
import { hostClasses, type HostClass } from "./host.ts";
import { parserOf } from "./schema.ts";

/** One competitor adapter of a campaign. */
export interface CampaignAdapter {
  readonly id: string;
  /** The program and its arguments, run in the campaign file's folder. */
  readonly command: readonly [string, ...string[]];
  /**
   * The tool's published configuration, a file beside the campaign's: what
   * it does by default where a Product has a control (ADR 0025).
   */
  readonly configuration?: string;
  /**
   * A matched row (ADR 0025's equal guarantees): the Product with the
   * controls the adapter `against` lacks switched off, by the settings
   * named here, and with Filter-list blocking and Consent prompt handling
   * set as that tool's published configuration declares its defaults.
   */
  readonly matched?: {
    readonly against: string;
    readonly settings: Readonly<Record<string, unknown>>;
  };
}

/** What a tool's published configuration declares it does by default (ADRs 0047 and 0048). */
export interface CompetitorDefaults {
  /** Whether it blocks requests by filter lists, as a blocker does. */
  readonly filter_list_blocking: boolean;
  /** Whether it answers or hides Consent prompts. */
  readonly consent_prompt_handling: boolean;
}

/** The row an adapter runs as, which `prepare` hands it. */
export type Row =
  | { readonly kind: "default" }
  | {
      readonly kind: "matched";
      readonly against: string;
      readonly settings: Readonly<Record<string, unknown>>;
      readonly competitor_defaults: CompetitorDefaults;
    };

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
          configuration: { type: "string", minLength: 1 },
          matched: {
            type: "object",
            additionalProperties: false,
            required: ["against", "settings"],
            properties: { against: { type: "string" }, settings: { type: "object" } },
          },
        },
      },
    },
    tasks: { type: "array", minItems: 1, items: { type: "string" } },
  },
});

/** A tool's published configuration, of which bench reads its defaults; the rest is the tool's. */
const publishedConfiguration = parserOf<{ readonly defaults: CompetitorDefaults }>({
  type: "object",
  required: ["defaults"],
  properties: {
    defaults: {
      type: "object",
      additionalProperties: false,
      required: ["filter_list_blocking", "consent_prompt_handling"],
      properties: {
        filter_list_blocking: { type: "boolean" },
        consent_prompt_handling: { type: "boolean" },
      },
    },
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
  /** Each adapter's row, by its ID. */
  readonly rows: ReadonlyMap<string, Row>;
}

/** Reads and checks the campaign file at `path`, and the published configurations its matched rows read. */
export async function loadCampaign(path: string): Promise<LoadedCampaign> {
  const absolute = resolve(path);
  const bytes = await readFile(absolute);
  const campaign = campaignFile.parse(JSON.parse(bytes.toString("utf8")), absolute);
  const folder = dirname(absolute);
  const rows = new Map<string, Row>();
  for (const adapter of campaign.adapters) {
    rows.set(adapter.id, await rowOf(adapter, campaign, folder, absolute));
  }
  return {
    campaign,
    path: absolute,
    folder,
    sha256: createHash("sha256").update(bytes).digest("hex"),
    rows,
  };
}

/** `adapter`'s row: matched against another adapter of `campaign` whose published configuration it reads. */
async function rowOf(
  adapter: CampaignAdapter,
  campaign: Campaign,
  folder: string,
  what: string,
): Promise<Row> {
  if (adapter.matched === undefined) {
    return { kind: "default" };
  }
  const { against, settings } = adapter.matched;
  const tool = campaign.adapters.find((other) => other.id === against && other !== adapter);
  if (tool?.configuration === undefined) {
    throw new BenchError(
      "invalid_document",
      `${what}: the adapter ${adapter.id} is matched against ${against}, which is no other adapter of the campaign with a published configuration`,
    );
  }
  const file = resolve(folder, tool.configuration);
  const { defaults } = publishedConfiguration.parse(JSON.parse(await readFile(file, "utf8")), file);
  return { kind: "matched", against, settings, competitor_defaults: defaults };
}
