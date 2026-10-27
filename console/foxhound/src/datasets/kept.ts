// What the offline cache keeps of a Dataset view a person keeps offline
// (spec #593, story 215; ADR 0031: Records are cached only when a person
// keeps a view offline, and secret fields never are). A kept view is built
// field by field from an allow-list: the Dataset's name, schema, key and
// counts, the columns the grid shows, and each Record read so far as its
// number and the values of those columns alone. Left out: each Record's
// URL, crawl request, commit time and Provenance, where a token in a query
// string, a payment or a certificate exception could sit, and every field
// the person did not show. The cache encrypts the row under the sign-in's
// key (`@goodfetch/console/offline`).
import type { DatasetRecord, DatasetSummary } from "@goodfetch/foxhound/generated";

/** A Record of a kept view: its number and the fields the view showed. */
export type KeptRecord = Pick<DatasetRecord, "number" | "value">;

/** A Dataset view kept offline: its run, its Dataset, the columns it showed and the Records it had read. */
export interface KeptView {
  readonly run_id: string;
  readonly dataset: DatasetSummary;
  readonly columns: readonly string[];
  readonly records: readonly KeptRecord[];
}

/**
 * The name the cache keeps the view of `dataset` of `runId` under, as its
 * row and as its Dataset, which an erasure of the Dataset forgets
 * (`forgetDataset`).
 */
export function keptDatasetOf(runId: string, dataset: string): string {
  return JSON.stringify([runId, dataset]);
}

/** What the cache keeps of the view of `dataset` showing `columns` of `records`. */
export function keptView(
  runId: string,
  dataset: DatasetSummary,
  columns: readonly string[],
  records: readonly DatasetRecord[],
): KeptView {
  return {
    run_id: runId,
    dataset: {
      name: dataset.name,
      ...(dataset.schema === undefined || dataset.schema === null
        ? {}
        : { schema: dataset.schema }),
      key: [...dataset.key],
      committed: dataset.committed,
      dropped: dataset.dropped,
      rejected: dataset.rejected,
    },
    columns: [...columns],
    records: records.map((record) => {
      const { value } = record;
      return {
        number: record.number,
        value: Object.fromEntries(
          columns
            .filter((column) => Object.hasOwn(value, column))
            .map((column) => [column, value[column]]),
        ),
      };
    }),
  };
}

/** Whether `value` reads as a view the cache kept for `runId`. */
export function isKeptViewOf(value: unknown, runId: string): value is KeptView {
  return (
    typeof value === "object" &&
    value !== null &&
    "run_id" in value &&
    value.run_id === runId &&
    "dataset" in value &&
    typeof value.dataset === "object" &&
    value.dataset !== null &&
    "columns" in value &&
    Array.isArray(value.columns) &&
    "records" in value &&
    Array.isArray(value.records)
  );
}
