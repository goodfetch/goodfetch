// The views a run that needs Foxhound has beside those every run has (ADRs
// 0087 and 0098), each named by the operation of Foxhound's catalog it
// reads, which the catalog's list of Foxhound's names types. Collie's build
// imports none of Foxhound's screens: on Collie's engine each view shows
// the `not_in_product` that operation answers, whose fix names Foxhound;
// Foxhound's build shows the views themselves.
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import type { FoxhoundOperation } from "#generated/foxhound.ts";

/** One view only Foxhound carries. */
export interface FoxhoundView {
  /** The operation of Foxhound's catalog it reads. */
  readonly operation: FoxhoundOperation;
  /** Its tab's name. */
  readonly name: MessageDescriptor;
}

/** A crawl's views, in their tabs' order. */
export const FOXHOUND_VIEWS: readonly FoxhoundView[] = [
  { operation: "run.records", name: msg`Records` },
  { operation: "run.outcomes", name: msg`Outcomes` },
  { operation: "run.failed_requests", name: msg`Failed requests` },
  { operation: "run.seeds", name: msg`Seeds` },
  { operation: "run.rejected_records", name: msg`Rejected records` },
];
