// The Runs screen's filter in the page's address (ADRs 0036 and 0117): the
// fields `run.list` takes, one query parameter each (`tag` once per tag,
// `started_from`, `started_until`, `ended_from` and `ended_until` for the
// two time ranges), so a link opens the same view. Reading the address
// keeps only values the engine takes: an unknown axis, sort key or a time
// that is no time is left out, never sent.
import type {
  Completeness,
  Lifecycle,
  Product,
  RunListParams,
  RunSortKey,
  RunTrigger,
  SortOrder,
  TimeRange,
  Verdict,
} from "@goodfetch/collie-client/generated";

/** The Lifecycles, in their order. */
export const LIFECYCLES: readonly Lifecycle[] = ["running", "stopped", "finished", "failed"];
/** The Verdicts, in their order. */
export const VERDICTS: readonly Verdict[] = ["verified", "rejected", "needs_review"];
/** The Completenesses, in their order. */
export const COMPLETENESSES: readonly Completeness[] = ["complete", "incomplete", "unverified"];
/** What fires a Schedule's occurrence. */
export const TRIGGERS: readonly RunTrigger[] = ["time", "watch", "after_run", "run_now"];
/** The Products. */
export const PRODUCTS: readonly Product[] = ["collie", "foxhound"];
/** What the rows sort by. */
export const SORT_KEYS: readonly RunSortKey[] = [
  "started_at",
  "ended_at",
  "run_id",
  "recipe",
  "schedule",
  "occurrence",
  "trigger",
  "product",
  "lifecycle",
  "verdict",
  "completeness",
];
/** Which way. */
export const ORDERS: readonly SortOrder[] = ["descending", "ascending"];

/** `value` when it is one of `known`. */
function oneOf<T extends string>(known: readonly T[], value: string | null): T | undefined {
  return known.find((candidate) => candidate === value);
}

/** `value` when it is not empty. */
function text(value: string | null): string | undefined {
  return value === null || value === "" ? undefined : value;
}

/** `value` when it is a point in time, as the engine writes it. */
function instant(value: string | null): string | undefined {
  return value !== null && Number.isFinite(Date.parse(value)) ? value : undefined;
}

/** The range `from` and `until` name, if either does. */
function range(from: string | undefined, until: string | undefined): TimeRange | undefined {
  if (from === undefined && until === undefined) {
    return undefined;
  }
  return {
    ...(from === undefined ? {} : { from }),
    ...(until === undefined ? {} : { until }),
  };
}

/** The filter the address's query `search` names. */
export function filterOfSearch(search: string): RunListParams {
  const query = new URLSearchParams(search);
  const tags = query.getAll("tag").filter((tag) => tag !== "");
  const filter: RunListParams = {};
  if (tags.length > 0) {
    filter.tags = tags;
  }
  const recipe = text(query.get("recipe"));
  if (recipe !== undefined) {
    filter.recipe = recipe;
  }
  const schedule = text(query.get("schedule"));
  if (schedule !== undefined) {
    filter.schedule = schedule;
  }
  const occurrence = text(query.get("occurrence"));
  if (occurrence !== undefined) {
    filter.occurrence = occurrence;
  }
  const trigger = oneOf(TRIGGERS, query.get("trigger"));
  if (trigger !== undefined) {
    filter.trigger = trigger;
  }
  const product = oneOf(PRODUCTS, query.get("product"));
  if (product !== undefined) {
    filter.product = product;
  }
  const lifecycle = oneOf(LIFECYCLES, query.get("lifecycle"));
  if (lifecycle !== undefined) {
    filter.lifecycle = lifecycle;
  }
  const verdict = oneOf(VERDICTS, query.get("verdict"));
  if (verdict !== undefined) {
    filter.verdict = verdict;
  }
  const completeness = oneOf(COMPLETENESSES, query.get("completeness"));
  if (completeness !== undefined) {
    filter.completeness = completeness;
  }
  const started = range(instant(query.get("started_from")), instant(query.get("started_until")));
  if (started !== undefined) {
    filter.started = started;
  }
  const ended = range(instant(query.get("ended_from")), instant(query.get("ended_until")));
  if (ended !== undefined) {
    filter.ended = ended;
  }
  const found = text(query.get("text"));
  if (found !== undefined) {
    filter.text = found;
  }
  if (query.get("watch_runs") === "true") {
    filter.watch_runs = true;
  }
  const sort = oneOf(SORT_KEYS, query.get("sort"));
  if (sort !== undefined) {
    filter.sort = sort;
  }
  const order = oneOf(ORDERS, query.get("order"));
  if (order !== undefined) {
    filter.order = order;
  }
  return filter;
}

/** The address's query that names `filter`: empty for no filter. */
export function searchOfFilter(filter: RunListParams): string {
  const query = new URLSearchParams();
  const set = (name: string, value: string | null | undefined): void => {
    if (value !== undefined && value !== null && value !== "") {
      query.append(name, value);
    }
  };
  for (const tag of filter.tags ?? []) {
    set("tag", tag);
  }
  set("recipe", filter.recipe);
  set("schedule", filter.schedule);
  set("occurrence", filter.occurrence);
  set("trigger", filter.trigger);
  set("product", filter.product);
  set("lifecycle", filter.lifecycle);
  set("verdict", filter.verdict);
  set("completeness", filter.completeness);
  set("started_from", filter.started?.from);
  set("started_until", filter.started?.until);
  set("ended_from", filter.ended?.from);
  set("ended_until", filter.ended?.until);
  set("text", filter.text);
  set("watch_runs", filter.watch_runs === true ? "true" : undefined);
  set("sort", filter.sort);
  set("order", filter.order);
  const search = query.toString();
  return search === "" ? "" : `?${search}`;
}
