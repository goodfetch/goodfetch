// The Runs screen (spec #591, stories 10 and 12 to 14; ADRs 0029, 0031,
// 0036 and 0117): every run of the user and major the Run index lists,
// whichever engine or Product ran it, as `run.list` answers it. The engine
// filters and sorts; the filter lives in the page's address, so a view can
// be shared, and Watch runs stay hidden unless it asks for them. A row whose
// run file is gone shows as missing and a run discarded on purpose as
// erased, never dropped. A live run's row follows its axes and status line
// from the event stream without reading the list again; a run the list does
// not hold, an axis change under a filter that names an axis, or a resync
// reads it again. Each row's Run ID links to the run's page.
import type {
  RunIndexRow,
  RunListParams,
  RunSortKey,
  SortOrder,
} from "@goodfetch/collie-client/generated";
import { Trans, useLingui } from "@lingui/react/macro";
import { useWindowVirtualizer } from "@tanstack/react-virtual";
import {
  useEffect,
  useId,
  useLayoutEffect,
  useReducer,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
  type RefObject,
  type SubmitEvent,
} from "react";
import type { Address } from "#address.ts";
import type { ConsoleClient, StreamItem } from "#client/client.ts";
import type { OfflineCache } from "#offline/offline.ts";
import { cachedRow } from "#offline/summaries.ts";
import { formatInstant, formatList } from "#format.ts";
import { ErrorMessage } from "#errors/error-message.tsx";
import { ReplayedMark } from "./run-report.tsx";
import { Owner } from "./owner.tsx";
import {
  COMPLETENESSES,
  LIFECYCLES,
  ORDERS,
  PRODUCTS,
  SORT_KEYS,
  TRIGGERS,
  VERDICTS,
  filterOfSearch,
  searchOfFilter,
} from "./filter.ts";
import {
  COMPLETENESS_NAMES,
  LIFECYCLE_NAMES,
  ORDER_NAMES,
  PRODUCT_NAMES,
  SORT_KEY_NAMES,
  TRIGGER_NAMES,
  VERDICT_NAMES,
} from "./names.ts";
import { Form } from "#form.tsx";
import { ScrollRegion } from "#scroll-region.tsx";

/** What the screen read of the Run index. */
export type Listed =
  | { readonly kind: "reading" }
  | { readonly kind: "listed"; readonly runs: readonly RunIndexRow[] }
  /** With no Connection yet, the runs the browser's offline cache keeps, newest first. */
  | { readonly kind: "cached"; readonly runs: readonly RunIndexRow[] }
  /** The engine refused, with this error. */
  | { readonly kind: "failed"; readonly error: unknown };

/**
 * What the event stream changed in listed rows since the list was read, by
 * Run ID: kept apart from the list, so an event costs as much with 10,000
 * runs listed as with ten (ADR 0071).
 */
export type RowChanges = ReadonlyMap<string, Partial<RunIndexRow>>;

/** No row changed. */
const UNCHANGED: RowChanges = new Map();

/** What the Runs screen shows. */
export interface RunsViewProps {
  readonly filter: RunListParams;
  readonly listed: Listed;
  /** What the event stream changed in the listed rows; none by default. */
  readonly changes?: RowChanges | undefined;
  /** Whether the Console has no Connection now, which disables filtering. */
  readonly readOnly?: boolean | undefined;
  readonly onFilter: (filter: RunListParams) => void;
}

/** What begins each tag only Goodfetch's own starters set, a Recipe's and the scheduler (ADR 0036). */
const RESERVED = "goodfetch.";

/** The value of the reserved tag `goodfetch.<key>:<value>` of `row`. */
function reserved(row: RunIndexRow, key: string): string | undefined {
  const prefix = `${RESERVED}${key}:`;
  return row.tags.find((tag) => tag.startsWith(prefix))?.slice(prefix.length);
}

/** The value `name` of the form, if it holds text. */
function textOf(form: FormData, name: string): string | undefined {
  const value = form.get(name);
  return typeof value === "string" && value.trim() !== "" ? value.trim() : undefined;
}

/** The value of a `datetime-local` field, as a point in time. */
function instantOf(form: FormData, name: string): string | undefined {
  const value = textOf(form, name);
  if (value === undefined) {
    return undefined;
  }
  const time = new Date(value);
  return Number.isFinite(time.getTime()) ? time.toISOString() : undefined;
}

/** `part`, in two digits at least. */
function pad(part: number): string {
  return String(part).padStart(2, "0");
}

/** A point in time as a `datetime-local` field shows it, on the browser's wall clock. */
function localValueOf(rfc3339: string | null | undefined): string {
  if (rfc3339 === undefined || rfc3339 === null) {
    return "";
  }
  const time = new Date(rfc3339);
  if (!Number.isFinite(time.getTime())) {
    return "";
  }
  return `${String(time.getFullYear())}-${pad(time.getMonth() + 1)}-${pad(time.getDate())}T${pad(time.getHours())}:${pad(time.getMinutes())}`;
}

/** The filter the form holds, in the address's form, so values it does not take are left out. */
function filterOfForm(form: FormData): RunListParams {
  const query = new URLSearchParams();
  const tags = (textOf(form, "tags") ?? "")
    .split(",")
    .map((tag) => tag.trim())
    .filter((tag) => tag !== "");
  for (const tag of tags) {
    query.append("tag", tag);
  }
  for (const name of [
    "text",
    "lifecycle",
    "verdict",
    "completeness",
    "recipe",
    "schedule",
    "occurrence",
    "trigger",
    "product",
    "sort",
    "order",
  ]) {
    const value = textOf(form, name);
    if (value !== undefined) {
      query.set(name, value);
    }
  }
  for (const name of ["started_from", "started_until", "ended_from", "ended_until"]) {
    const value = instantOf(form, name);
    if (value !== undefined) {
      query.set(name, value);
    }
  }
  if (form.get("watch_runs") === "true") {
    query.set("watch_runs", "true");
  }
  // The engine's own order needs no word in the address.
  if (query.get("sort") === DEFAULT_SORT) {
    query.delete("sort");
  }
  if (query.get("order") === DEFAULT_ORDER) {
    query.delete("order");
  }
  return filterOfSearch(query.toString());
}

/** What the engine sorts by when the filter names nothing, and which way (`run.list`). */
const DEFAULT_SORT: RunSortKey = "started_at";
const DEFAULT_ORDER: SortOrder = "descending";

/** A field of the filter: its label above its control. */
function Field({
  label,
  children,
  id,
}: {
  readonly label: ReactNode;
  readonly children: ReactNode;
  readonly id: string;
}): ReactNode {
  return (
    <div className="flex flex-col gap-1">
      <label htmlFor={id} className="text-sm font-medium">
        {label}
      </label>
      {children}
    </div>
  );
}

const controlClassName =
  "rounded-md border bg-background px-2 py-1 focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none";

/** A choice among `values`, named by `names`, or none. */
function Choice<T extends string>({
  id,
  name,
  values,
  nameOf,
  value,
  any,
}: {
  readonly id: string;
  readonly name: string;
  readonly values: readonly T[];
  /** Each value's name. */
  readonly nameOf: (value: T) => string;
  readonly value: T | null | undefined;
  /** The name of choosing none, if none may be chosen. */
  readonly any?: string | undefined;
}): ReactNode {
  return (
    <select id={id} name={name} defaultValue={value ?? ""} className={controlClassName}>
      {any === undefined ? null : <option value="">{any}</option>}
      {values.map((candidate) => (
        <option key={candidate} value={candidate}>
          {nameOf(candidate)}
        </option>
      ))}
    </select>
  );
}

/** Whether a filter names any field the "More filters" group holds. */
function namesMore(filter: RunListParams): boolean {
  return (
    filter.recipe != null ||
    filter.schedule != null ||
    filter.occurrence != null ||
    filter.trigger != null ||
    filter.product != null ||
    filter.started != null ||
    filter.ended != null ||
    filter.watch_runs === true
  );
}

/** The filter's form. */
function FilterForm({
  filter,
  readOnly,
  onFilter,
}: {
  readonly filter: RunListParams;
  readonly readOnly: boolean;
  readonly onFilter: (filter: RunListParams) => void;
}): ReactNode {
  const { i18n, t } = useLingui();
  const id = useId();
  const [more, setMore] = useState(() => namesMore(filter));
  const submit = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    onFilter(filterOfForm(new FormData(event.currentTarget)));
  };
  return (
    <search aria-label={t`Filter runs`}>
      <Form onSubmit={submit} className="flex flex-col gap-3 rounded-lg border p-4">
        <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <Field id={`${id}-text`} label={<Trans>Search</Trans>}>
            <input
              id={`${id}-text`}
              name="text"
              type="search"
              defaultValue={filter.text ?? ""}
              className={controlClassName}
            />
          </Field>
          <Field id={`${id}-lifecycle`} label={<Trans>Lifecycle</Trans>}>
            <Choice
              id={`${id}-lifecycle`}
              name="lifecycle"
              values={LIFECYCLES}
              nameOf={(value) => i18n._(LIFECYCLE_NAMES[value])}
              value={filter.lifecycle}
              any={t`Any`}
            />
          </Field>
          <Field id={`${id}-verdict`} label={<Trans>Verdict</Trans>}>
            <Choice
              id={`${id}-verdict`}
              name="verdict"
              values={VERDICTS}
              nameOf={(value) => i18n._(VERDICT_NAMES[value])}
              value={filter.verdict}
              any={t`Any`}
            />
          </Field>
          <Field id={`${id}-completeness`} label={<Trans>Completeness</Trans>}>
            <Choice
              id={`${id}-completeness`}
              name="completeness"
              values={COMPLETENESSES}
              nameOf={(value) => i18n._(COMPLETENESS_NAMES[value])}
              value={filter.completeness}
              any={t`Any`}
            />
          </Field>
          <Field id={`${id}-tags`} label={<Trans>Tags</Trans>}>
            <input
              id={`${id}-tags`}
              name="tags"
              type="text"
              defaultValue={(filter.tags ?? []).join(", ")}
              aria-describedby={`${id}-tags-hint`}
              className={controlClassName}
            />
            <p id={`${id}-tags-hint`} className="text-sm text-muted-foreground">
              <Trans>Separated by commas; a run carries every one.</Trans>
            </p>
          </Field>
          <Field id={`${id}-sort`} label={<Trans>Sort by</Trans>}>
            <Choice
              id={`${id}-sort`}
              name="sort"
              values={SORT_KEYS}
              nameOf={(value) => i18n._(SORT_KEY_NAMES[value])}
              value={filter.sort}
            />
          </Field>
          <Field id={`${id}-order`} label={<Trans>Order</Trans>}>
            <Choice
              id={`${id}-order`}
              name="order"
              values={ORDERS}
              nameOf={(value) => i18n._(ORDER_NAMES[value])}
              value={filter.order}
            />
          </Field>
        </div>
        <button
          type="button"
          aria-expanded={more}
          aria-controls={`${id}-more`}
          onClick={() => setMore((open) => !open)}
          className="self-start rounded-sm font-medium underline-offset-4 hover:underline focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none"
        >
          <Trans>More filters</Trans>
        </button>
        {/* Hidden, its fields are still the form's, so a filter they hold is kept. */}
        <div id={`${id}-more`} hidden={!more}>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 lg:grid-cols-4">
            <Field id={`${id}-recipe`} label={<Trans>Recipe</Trans>}>
              <input
                id={`${id}-recipe`}
                name="recipe"
                type="text"
                defaultValue={filter.recipe ?? ""}
                className={controlClassName}
              />
            </Field>
            <Field id={`${id}-schedule`} label={<Trans>Schedule</Trans>}>
              <input
                id={`${id}-schedule`}
                name="schedule"
                type="text"
                defaultValue={filter.schedule ?? ""}
                className={controlClassName}
              />
            </Field>
            <Field id={`${id}-occurrence`} label={<Trans>Occurrence</Trans>}>
              <input
                id={`${id}-occurrence`}
                name="occurrence"
                type="text"
                defaultValue={filter.occurrence ?? ""}
                className={controlClassName}
              />
            </Field>
            <Field id={`${id}-trigger`} label={<Trans>Trigger</Trans>}>
              <Choice
                id={`${id}-trigger`}
                name="trigger"
                values={TRIGGERS}
                nameOf={(value) => i18n._(TRIGGER_NAMES[value])}
                value={filter.trigger}
                any={t`Any`}
              />
            </Field>
            <Field id={`${id}-product`} label={<Trans>Product</Trans>}>
              <Choice
                id={`${id}-product`}
                name="product"
                values={PRODUCTS}
                nameOf={(value) => PRODUCT_NAMES[value]}
                value={filter.product}
                any={t`Any`}
              />
            </Field>
            <Field id={`${id}-started-from`} label={<Trans>Started from</Trans>}>
              <input
                id={`${id}-started-from`}
                name="started_from"
                type="datetime-local"
                defaultValue={localValueOf(filter.started?.from)}
                className={controlClassName}
              />
            </Field>
            <Field id={`${id}-started-until`} label={<Trans>Started before</Trans>}>
              <input
                id={`${id}-started-until`}
                name="started_until"
                type="datetime-local"
                defaultValue={localValueOf(filter.started?.until)}
                className={controlClassName}
              />
            </Field>
            <Field id={`${id}-ended-from`} label={<Trans>Ended from</Trans>}>
              <input
                id={`${id}-ended-from`}
                name="ended_from"
                type="datetime-local"
                defaultValue={localValueOf(filter.ended?.from)}
                className={controlClassName}
              />
            </Field>
            <Field id={`${id}-ended-until`} label={<Trans>Ended before</Trans>}>
              <input
                id={`${id}-ended-until`}
                name="ended_until"
                type="datetime-local"
                defaultValue={localValueOf(filter.ended?.until)}
                className={controlClassName}
              />
            </Field>
            <div className="flex items-center gap-2">
              <input
                id={`${id}-watch-runs`}
                name="watch_runs"
                type="checkbox"
                value="true"
                defaultChecked={filter.watch_runs === true}
              />
              <label htmlFor={`${id}-watch-runs`}>
                <Trans>Show Watch runs</Trans>
              </label>
            </div>
          </div>
        </div>
        <button
          type="submit"
          disabled={readOnly}
          className="self-start rounded-md bg-primary px-3 py-1.5 font-medium text-primary-foreground focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50"
        >
          <Trans>Apply</Trans>
        </button>
      </Form>
    </search>
  );
}

/** A point in time, with its machine form beside it. */
function Instant({ value }: { readonly value: string | null | undefined }): ReactNode {
  if (value === undefined || value === null) {
    return null;
  }
  return (
    <time dateTime={value} translate="no">
      {formatInstant(value)}
    </time>
  );
}

/** The mark of a run's presence, when its state is not there. */
function Presence({ row }: { readonly row: RunIndexRow }): ReactNode {
  switch (row.presence) {
    case "present":
      return null;
    case "missing":
      return (
        <span className="block text-sm font-medium text-destructive">
          <Trans>Run file missing</Trans>
        </span>
      );
    case "erased":
      return (
        <span className="block text-sm font-medium text-muted-foreground">
          <Trans>Erased</Trans>
        </span>
      );
    default:
      return row.presence satisfies never;
  }
}

/** One run's row, the `index`th of the list, which `measure` sizes once rendered. */
function RunRow({
  row,
  index,
  measure,
}: {
  readonly row: RunIndexRow;
  readonly index: number;
  readonly measure: (row: HTMLTableRowElement | null) => void;
}): ReactNode {
  const { i18n } = useLingui();
  const tags = row.tags.filter((tag) => !tag.startsWith(RESERVED));
  const watch = reserved(row, "role") === "watch";
  const occurrence = reserved(row, "occurrence");
  return (
    <tr
      ref={measure}
      data-index={index}
      // The header row is the table's first.
      aria-rowindex={index + 2}
      className="border-b align-top"
    >
      <th scope="row" className="py-2 pe-4 text-start font-normal">
        <a
          href={`/runs/${encodeURIComponent(row.run_id)}`}
          className="rounded-sm underline-offset-4 hover:underline focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none"
        >
          <code translate="no" className="font-mono">
            {row.run_id}
          </code>
        </a>
        <Presence row={row} />
        {row.replayed ? (
          <span className="block">
            <ReplayedMark />
          </span>
        ) : null}
        {watch ? (
          <span className="block text-sm text-muted-foreground">
            <Trans>Watch run</Trans>
          </span>
        ) : null}
        {tags.length === 0 ? null : (
          <span className="block text-sm text-muted-foreground" translate="no">
            {formatList(tags)}
          </span>
        )}
      </th>
      <td className="py-2 pe-4">{i18n._(LIFECYCLE_NAMES[row.lifecycle])}</td>
      <td className="py-2 pe-4">
        {row.verdict === undefined || row.verdict === null
          ? null
          : i18n._(VERDICT_NAMES[row.verdict])}
      </td>
      <td className="py-2 pe-4">
        {row.completeness === undefined || row.completeness === null
          ? null
          : i18n._(COMPLETENESS_NAMES[row.completeness])}
      </td>
      <td className="py-2 pe-4" translate="no">
        {PRODUCT_NAMES[row.needs]}
      </td>
      <td className="py-2 pe-4">
        {row.lifecycle === "running" && row.owner !== undefined && row.owner !== null ? (
          <Owner owner={row.owner} />
        ) : null}
      </td>
      <td className="py-2 pe-4" translate="no">
        {reserved(row, "recipe")}
      </td>
      <td className="py-2 pe-4" translate="no">
        {reserved(row, "schedule")}
        {occurrence === undefined ? null : (
          <span className="block text-sm text-muted-foreground">{occurrence}</span>
        )}
      </td>
      <td className="py-2 pe-4">
        <Instant value={row.started_at} />
      </td>
      <td className="py-2 pe-4">
        <Instant value={row.ended_at} />
      </td>
      <td className="py-2" translate="no">
        {row.progress_message}
      </td>
    </tr>
  );
}

/** The runs the screen read, as a table. */
function RunTable({
  runs,
  changes,
}: {
  readonly runs: readonly RunIndexRow[];
  readonly changes: RowChanges;
}): ReactNode {
  if (runs.length === 0) {
    return (
      <output className="block text-muted-foreground">
        <Trans>No run matches this filter.</Trans>
      </output>
    );
  }
  return <WindowedRunTable runs={runs} changes={changes} />;
}

/**
 * The room of the rows not rendered at one end of the table, so the page
 * keeps its height. Hidden from assistive technology, which counts rows by
 * the table's `aria-rowcount`.
 */
function Spacer({ height }: { readonly height: number }): ReactNode {
  return <tr aria-hidden="true" style={{ height }} />;
}

/** A row's height before it is measured, in CSS pixels: one line and its padding. */
const ROW_ESTIMATE = 41;

/** The rows rendered beyond each edge of the window, so scrolling shows no gap. */
const OVERSCAN = 8;

/**
 * The page's offset of `element`'s top, in CSS pixels, kept as what is above
 * it changes height (a filter group opened, a banner shown).
 */
function useDocumentOffset(element: RefObject<HTMLElement | null>): number {
  const [offset, setOffset] = useState(0);
  useLayoutEffect(() => {
    const measured = element.current;
    if (measured === null) {
      return undefined;
    }
    const measure = (): void => setOffset(measured.getBoundingClientRect().top + window.scrollY);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(document.body);
    return () => observer.disconnect();
  }, [element]);
  return offset;
}

/**
 * A non-empty list of runs, rendering only the rows in the window and a few
 * beyond (ADR 0071: a 10,000-row list stays smooth). The page scrolls, not
 * the table, and the table names how many rows it has for assistive
 * technology, each row its place.
 */
function WindowedRunTable({
  runs,
  changes,
}: {
  readonly runs: readonly RunIndexRow[];
  readonly changes: RowChanges;
}): ReactNode {
  const { t } = useLingui();
  const body = useRef<HTMLTableSectionElement>(null);
  const scrollMargin = useDocumentOffset(body);
  const rows = useWindowVirtualizer({
    count: runs.length,
    estimateSize: () => ROW_ESTIMATE,
    overscan: OVERSCAN,
    scrollMargin,
    getItemKey: (index) => runs[index]?.run_id ?? index,
  });
  const shown = rows.getVirtualItems();
  const before = (shown[0]?.start ?? scrollMargin) - scrollMargin;
  const after = rows.getTotalSize() - ((shown.at(-1)?.end ?? scrollMargin) - scrollMargin);
  return (
    <ScrollRegion label={t`Runs, scrollable`}>
      <table
        aria-label={t`Runs`}
        aria-rowcount={runs.length + 1}
        className="w-full border-collapse text-start"
      >
        <thead>
          <tr className="border-b">
            <th scope="col" className="py-2 pe-4">
              <Trans>Run</Trans>
            </th>
            <th scope="col" className="py-2 pe-4">
              <Trans>Lifecycle</Trans>
            </th>
            <th scope="col" className="py-2 pe-4">
              <Trans>Verdict</Trans>
            </th>
            <th scope="col" className="py-2 pe-4">
              <Trans>Completeness</Trans>
            </th>
            <th scope="col" className="py-2 pe-4">
              <Trans>Product</Trans>
            </th>
            <th scope="col" className="py-2 pe-4">
              <Trans>Owning engine</Trans>
            </th>
            <th scope="col" className="py-2 pe-4">
              <Trans>Recipe</Trans>
            </th>
            <th scope="col" className="py-2 pe-4">
              <Trans>Schedule</Trans>
            </th>
            <th scope="col" className="py-2 pe-4">
              <Trans>Started</Trans>
            </th>
            <th scope="col" className="py-2 pe-4">
              <Trans>Ended</Trans>
            </th>
            <th scope="col" className="py-2">
              <Trans>Status</Trans>
            </th>
          </tr>
        </thead>
        <tbody ref={body}>
          {before > 0 ? <Spacer height={before} /> : null}
          {shown.map((item) => {
            const run = runs[item.index];
            if (run === undefined) {
              return null;
            }
            const change = changes.get(run.run_id);
            return (
              <RunRow
                key={item.key}
                row={change === undefined ? run : { ...run, ...change }}
                index={item.index}
                measure={rows.measureElement}
              />
            );
          })}
          {after > 0 ? <Spacer height={after} /> : null}
        </tbody>
      </table>
    </ScrollRegion>
  );
}

/** What the screen read, shown. */
function ListedRuns({
  listed,
  changes,
}: {
  readonly listed: Listed;
  readonly changes: RowChanges;
}): ReactNode {
  switch (listed.kind) {
    case "reading":
      return (
        <output className="block">
          <Trans>Reading the runs…</Trans>
        </output>
      );
    case "failed":
      return (
        <div role="alert">
          <p>
            <Trans>The engine could not list the runs:</Trans>
          </p>
          <ErrorMessage error={listed.error} />
        </div>
      );
    case "listed":
      return <RunTable runs={listed.runs} changes={changes} />;
    case "cached":
      return (
        <>
          <output className="block">
            <Trans>
              This browser's offline copy of the runs it last listed, newest first: the filter
              applies, and every run shows, once the Console reconnects.
            </Trans>
          </output>
          <RunTable runs={listed.runs} changes={UNCHANGED} />
        </>
      );
    default:
      return listed satisfies never;
  }
}

/** The Runs screen, as it shows `listed` under `filter`. */
export function RunsView({
  filter,
  listed,
  changes = UNCHANGED,
  readOnly = false,
  onFilter,
}: RunsViewProps): ReactNode {
  return (
    <div className="flex flex-col gap-6">
      <h1 className="font-heading text-3xl font-semibold tracking-tight">
        <Trans>Runs</Trans>
      </h1>
      <FilterForm filter={filter} readOnly={readOnly} onFilter={onFilter} />
      <ListedRuns listed={listed} changes={changes} />
    </div>
  );
}

/** What the screen holds: its filter, what it read under it, and how many reads it asked for. */
interface RunsState {
  readonly filter: RunListParams;
  readonly listed: Listed;
  /** The Run IDs of the rows listed. */
  readonly ids: ReadonlySet<string>;
  /** What the event stream changed in them since. */
  readonly changes: RowChanges;
  /** Counts the reads asked for besides a new filter: a change the list cannot follow by itself. */
  readonly reads: number;
}

type RunsAction =
  | { readonly kind: "filtered"; readonly filter: RunListParams }
  | { readonly kind: "read"; readonly listed: Listed }
  /** The offline cache's runs, shown only until the engine answers. */
  | { readonly kind: "cached"; readonly runs: readonly RunIndexRow[] }
  | { readonly kind: "streamed"; readonly item: StreamItem };

/** The field `name` of `value`, if it is an object that has one. */
function member(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

/** `value` when it is one of `known`. */
function known<T extends string>(values: readonly T[], value: unknown): T | undefined {
  return values.find((candidate) => candidate === value);
}

/** The state with `change` made to the listed row of `runId`. */
function withChange(state: RunsState, runId: string, change: Partial<RunIndexRow>): RunsState {
  const changes = new Map(state.changes);
  changes.set(runId, { ...changes.get(runId), ...change });
  return { ...state, changes };
}

/** The state after an event of the stream. */
function streamed(state: RunsState, item: StreamItem): RunsState {
  if (item.kind === "resync") {
    return { ...state, reads: state.reads + 1 };
  }
  if (state.listed.kind !== "listed") {
    return state;
  }
  const runId = member(item.data, "run_id");
  if (typeof runId !== "string") {
    return state;
  }
  const listed = state.ids.has(runId);
  if (item.event === "run.progress") {
    const message = member(item.data, "message");
    return listed
      ? withChange(state, runId, { progress_message: typeof message === "string" ? message : null })
      : state;
  }
  if (item.event !== "run.axes_changed") {
    return state;
  }
  const lifecycle = known(LIFECYCLES, member(item.data, "lifecycle"));
  if (lifecycle === undefined) {
    return state;
  }
  if (!listed) {
    return { ...state, reads: state.reads + 1 };
  }
  const next = withChange(state, runId, {
    lifecycle,
    verdict: known(VERDICTS, member(item.data, "verdict")) ?? null,
    completeness: known(COMPLETENESSES, member(item.data, "completeness")) ?? null,
  });
  const { filter } = state;
  const namesAnAxis =
    filter.lifecycle != null || filter.verdict != null || filter.completeness != null;
  return namesAnAxis ? { ...next, reads: next.reads + 1 } : next;
}

function reduce(state: RunsState, action: RunsAction): RunsState {
  switch (action.kind) {
    case "filtered":
      return { ...state, filter: action.filter };
    case "read":
      return {
        ...state,
        listed: action.listed,
        ids:
          action.listed.kind === "listed"
            ? new Set(action.listed.runs.map((row) => row.run_id))
            : new Set(),
        changes: UNCHANGED,
      };
    case "cached":
      return state.listed.kind === "reading"
        ? { ...state, listed: { kind: "cached", runs: action.runs } }
        : state;
    case "streamed":
      return streamed(state, action.item);
    default:
      return action satisfies never;
  }
}

/** Whether `value` reads as a Run index row the offline cache kept. */
function isRow(value: unknown): value is RunIndexRow {
  return typeof member(value, "run_id") === "string" && Array.isArray(member(value, "tags"));
}

/** The rows the offline cache keeps, newest first. */
async function cachedRuns(offline: OfflineCache): Promise<RunIndexRow[]> {
  const kept = (await offline.all("run_summary")).filter(isRow);
  return kept.toSorted((left, right) => right.started_at.localeCompare(left.started_at));
}

/** The Runs screen over `client`, its filter kept in `address`, each run it lists kept in `offline`. */
export function Runs({
  client,
  address,
  offline,
}: {
  readonly client: ConsoleClient;
  readonly address: Address;
  readonly offline?: OfflineCache | undefined;
}): ReactNode {
  const connection = useSyncExternalStore(
    (changed) => client.watch(changed),
    () => client.state,
  );
  const [state, dispatch] = useReducer(reduce, address, (initial): RunsState => ({
    filter: filterOfSearch(initial.search),
    listed: { kind: "reading" },
    ids: new Set(),
    changes: UNCHANGED,
    reads: 0,
  }));
  const { filter, reads } = state;

  useEffect(() => {
    if (connection !== "connected") {
      return undefined;
    }
    let current = true;
    const read = async (): Promise<void> => {
      let listed: Listed;
      try {
        listed = { kind: "listed", runs: (await client.call("run.list", filter)).runs };
        for (const row of listed.runs) {
          void offline?.put("run_summary", row.run_id, cachedRow(row));
        }
      } catch (error) {
        listed = { kind: "failed", error };
      }
      if (current) {
        dispatch({ kind: "read", listed });
      }
    };
    void read();
    return () => {
      current = false;
    };
  }, [client, connection, filter, reads, offline]);

  const waiting = state.listed.kind === "reading";
  useEffect(() => {
    if (connection === "connected" || !waiting || offline === undefined) {
      return undefined;
    }
    let current = true;
    const show = async (): Promise<void> => {
      const runs = await cachedRuns(offline);
      if (current && runs.length > 0) {
        dispatch({ kind: "cached", runs });
      }
    };
    void show();
    return () => {
      current = false;
    };
  }, [connection, waiting, offline]);

  useEffect(() => {
    const subscription = client.subscribe("events");
    const follow = async (): Promise<void> => {
      for await (const item of subscription) {
        dispatch({ kind: "streamed", item });
      }
    };
    void follow();
    return () => subscription.close();
  }, [client]);

  return (
    <RunsView
      filter={filter}
      listed={state.listed}
      changes={state.changes}
      readOnly={connection !== "connected"}
      onFilter={(next) => {
        address.replace(searchOfFilter(next));
        dispatch({ kind: "filtered", filter: next });
      }}
    />
  );
}
