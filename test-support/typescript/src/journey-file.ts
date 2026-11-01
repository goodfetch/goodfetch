// A Journey file (ADR 0063), as the TypeScript Journey helpers read it:
// parsed with `smol-toml` and validated against the committed
// `journeys/journey.schema.json`, which `xtask codegen` writes from
// `goodfetch-protocol`'s `Journey` type. The helpers read only what a run
// records by; the catalog check holds the rest of the file.
import { readFileSync } from "node:fs";
import type { ValidateFunction } from "ajv/dist/2020.js";
import { parse } from "smol-toml";
import { TestSupportError } from "./errors.ts";
import type { Product } from "./product-env.ts";
import { ajv } from "./schemas.ts";

/** The catalog's folder, `journeys/` at the repository's root. */
export const JOURNEY_CATALOG: URL = new URL("../../../journeys/", import.meta.url);

/** One step of a Journey. */
export interface JourneyStep {
  readonly text: string;
  /** The surface the step runs on in every variant, when pinned. */
  readonly on?: string | null | undefined;
  /** Why only a person can take the step, for a manual one. */
  readonly manual?: string | null | undefined;
}

/** One check of a Journey's outcome. */
export interface JourneyCheck {
  readonly text: string;
  /** The Proving ground task the oracle must answer `verified` for. */
  readonly task?: string | null | undefined;
  /** The surfaces of the variants the check belongs to; every variant's when absent or empty. */
  readonly surfaces?: readonly string[] | undefined;
}

/** Whether `check` belongs to the variant on `surface`, a `foxhound-cli` variant taking the `cli` variant's checks. */
export function belongsTo(check: JourneyCheck, surface: string): boolean {
  const surfaces = check.surfaces ?? [];
  return (
    surfaces.length === 0 ||
    surfaces.some(
      (listed) => listed === surface || (surface === "foxhound-cli" && listed === "cli"),
    )
  );
}

/** What a Journey helper reads of a Journey file. */
export interface JourneyFile {
  readonly id: string;
  readonly title: string;
  /** The Product whose executable the Journey drives (ADR 0094). */
  readonly product: Product;
  /** Its Proving ground's seed and the tasks it names; none when its goal touches no page. */
  readonly proving_ground?:
    | { readonly seed: number; readonly tasks?: readonly { readonly id: string }[] | undefined }
    | null
    | undefined;
  /** Each surface it has a variant on, with the milestone that variant is due at. */
  readonly surfaces: Readonly<Record<string, string>>;
  readonly steps: readonly JourneyStep[];
  readonly expect?: readonly JourneyCheck[] | undefined;
}

const isJourneyFile: ValidateFunction<JourneyFile> = ajv.compile<JourneyFile>(
  JSON.parse(readFileSync(new URL("journey.schema.json", JOURNEY_CATALOG), "utf8")),
);

/**
 * Reads the Journey file at `path`.
 *
 * @throws {TestSupportError} `journey_invalid` when it cannot be read, is not
 * TOML, or fails `journeys/journey.schema.json`.
 */
export function readJourneyFile(path: string | URL): JourneyFile {
  let journey: unknown;
  try {
    journey = parse(readFileSync(path, "utf8"));
  } catch (cause) {
    throw new TestSupportError(
      "journey_invalid",
      `the Journey file ${String(path)} cannot be read as TOML`,
      {
        cause,
      },
    );
  }
  if (!isJourneyFile(journey)) {
    throw new TestSupportError(
      "journey_invalid",
      `the Journey file ${String(path)} fails journeys/journey.schema.json: ${ajv.errorsText(isJourneyFile.errors)}`,
    );
  }
  return journey;
}

/** The tasks a Journey names, in its order. */
export function namedTasks(journey: JourneyFile): string[] {
  return (journey.proving_ground?.tasks ?? []).map(({ id }) => id);
}
