// What the Runs screen calls each value of a run's axes, what fired its
// occurrence and each sort key (ADR 0029: three axes, each its own word),
// in the Console's catalog, and each Product, by its name, which no
// language translates (ADR 0090).
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import type {
  Completeness,
  Lifecycle,
  Product,
  RunSortKey,
  RunTrigger,
  SortOrder,
  Verdict,
} from "@goodfetch/collie-client/generated";

export const LIFECYCLE_NAMES: Readonly<Record<Lifecycle, MessageDescriptor>> = {
  running: msg`running`,
  stopped: msg`stopped`,
  finished: msg`finished`,
  failed: msg`failed`,
};

export const VERDICT_NAMES: Readonly<Record<Verdict, MessageDescriptor>> = {
  verified: msg`verified`,
  rejected: msg`rejected`,
  needs_review: msg`needs review`,
};

export const COMPLETENESS_NAMES: Readonly<Record<Completeness, MessageDescriptor>> = {
  complete: msg`complete`,
  incomplete: msg`incomplete`,
  unverified: msg`unverified`,
};

/** Each Product's name, which no language translates (ADR 0090). */
export const PRODUCT_NAMES: Readonly<Record<Product, string>> = {
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- a Product's name (ADR 0090).
  collie: "Collie",
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- a Product's name (ADR 0090).
  foxhound: "Foxhound",
};

export const TRIGGER_NAMES: Readonly<Record<RunTrigger, MessageDescriptor>> = {
  time: msg`time`,
  watch: msg`Watch`,
  after_run: msg`after a run`,
  run_now: msg`run now`,
};

export const SORT_KEY_NAMES: Readonly<Record<RunSortKey, MessageDescriptor>> = {
  started_at: msg`Start`,
  ended_at: msg`End`,
  run_id: msg`Run ID`,
  recipe: msg`Recipe`,
  schedule: msg`Schedule`,
  occurrence: msg`Occurrence`,
  trigger: msg`Trigger`,
  product: msg`Product`,
  lifecycle: msg`Lifecycle`,
  verdict: msg`Verdict`,
  completeness: msg`Completeness`,
};

export const ORDER_NAMES: Readonly<Record<SortOrder, MessageDescriptor>> = {
  descending: msg`Descending`,
  ascending: msg`Ascending`,
};
