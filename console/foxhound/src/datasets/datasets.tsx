// A crawl run's Datasets (spec #593, stories 204, 205, 207 and 233; ADRs
// 0031, 0071, 0073 and 0103): the Datasets its crawl declares, each with
// its JSON Schema, its key and how many Records were committed, dropped
// and rejected, as `dataset.list` answers; and one Dataset's Records in a
// grid read page by page with `dataset.page`, whose filters (the `wall`
// marker included), sort and columns the engine applies, so a Dataset of
// millions of Records opens at once. The grid renders only the rows on
// screen and a few beyond, and reads the next page once its last loaded
// rows come into view. Each cell is isolated with `dir="auto"` and kept
// out of browser translation, so mixed-direction values read correctly.
// With the page's offline cache, "Keep offline" keeps the view's Records
// as the grid shows them (`kept.ts`, story 215), and a view opened with no
// Connection shows the run's kept views until the engine answers.
import type { RunViewProps } from "@goodfetch/console/console-build";
import { ErrorMessage } from "@goodfetch/console/error-message";
import type {
  DatasetPageParams,
  DatasetRecord,
  DatasetSummary,
  RecordFilter,
  WallFilter,
} from "@goodfetch/foxhound/generated";
import { Plural, Trans, useLingui } from "@lingui/react/macro";
import { useWindowVirtualizer } from "@tanstack/react-virtual";
import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type SubmitEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { callFoxhound } from "#call.ts";
import {
  isKeptViewOf,
  keptDatasetOf,
  keptView,
  type KeptRecord,
  type KeptView,
} from "#datasets/kept.ts";

/** How many Records the grid reads per page. */
export const RECORDS_PAGE = 500;

/** What the person asked the engine for: the sort, the filters and the columns kept. */
export interface RecordsQuery {
  readonly sort?: { readonly field: string; readonly direction: "ascending" | "descending" };
  readonly contains?: { readonly field: string; readonly text: string };
  readonly wall?: WallFilter;
  /** The fields shown; none shows every field. */
  readonly columns?: readonly string[];
}

/** The params of the page of `dataset` after `after`, through `through`, under `query`. */
export function pageParams(
  runId: string,
  dataset: string,
  query: RecordsQuery,
  after?: { readonly after: number; readonly through: number },
): DatasetPageParams {
  const filter: RecordFilter = {
    ...(query.contains === undefined
      ? {}
      : { fields: [{ field: query.contains.field, op: "contains", text: query.contains.text }] }),
    ...(query.wall === undefined ? {} : { wall: query.wall }),
  };
  return {
    run_id: runId,
    dataset,
    limit: RECORDS_PAGE,
    ...after,
    ...(query.sort === undefined ? {} : { sort: { ...query.sort } }),
    ...(Object.keys(filter).length === 0 ? {} : { filter }),
    ...(query.columns === undefined ? {} : { columns: [...query.columns] }),
  };
}

/** The top-level fields of `dataset`'s schema, else of the Records read. */
export function fieldsOf(
  dataset: DatasetSummary,
  records: readonly DatasetRecord[],
): readonly string[] {
  const properties = dataset.schema?.["properties"];
  if (typeof properties === "object" && properties !== null && !Array.isArray(properties)) {
    return Object.keys(properties);
  }
  const fields = new Set<string>();
  for (const record of records) {
    if (typeof record.value === "object" && record.value !== null) {
      for (const field of Object.keys(record.value)) {
        fields.add(field);
      }
    }
  }
  return [...fields];
}

/** A value as a cell shows it: text as it is, any other JSON as JSON, nothing when absent. */
function cellText(value: unknown): string {
  if (value === undefined) {
    return "";
  }
  return typeof value === "string" ? value : JSON.stringify(value);
}

const cellClassName = "border-b px-2 py-1 text-start align-top";

/** The Datasets a crawl declares, each with its schema, its key and its counts. */
export function DatasetList({
  datasets,
}: {
  readonly datasets: readonly DatasetSummary[];
}): ReactNode {
  const { t } = useLingui();
  // Counts read in the browser's own language, as the Console formats every number.
  const counts = new Intl.NumberFormat();
  return (
    <table aria-label={t`Datasets`} className="w-full text-start">
      <thead>
        <tr>
          <th scope="col" className={cellClassName}>
            <Trans>Dataset</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Schema</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Key</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Committed</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Dropped</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Rejected</Trans>
          </th>
        </tr>
      </thead>
      <tbody>
        {datasets.map((dataset) => (
          <tr key={dataset.name}>
            <th scope="row" className={cellClassName} dir="auto" translate="no">
              {dataset.name}
            </th>
            <td className={`${cellClassName} break-all`}>
              {dataset.schema === undefined || dataset.schema === null ? (
                <Trans>Any JSON object</Trans>
              ) : (
                <code dir="ltr" translate="no">
                  {JSON.stringify(dataset.schema)}
                </code>
              )}
            </td>
            <td className={cellClassName}>
              {dataset.key.length === 0 ? (
                <Trans>None</Trans>
              ) : (
                <span dir="auto" translate="no">
                  {dataset.key.join(", ")}
                </span>
              )}
            </td>
            <td className={cellClassName}>{counts.format(dataset.committed)}</td>
            <td className={cellClassName}>{counts.format(dataset.dropped)}</td>
            <td className={cellClassName}>{counts.format(dataset.rejected)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** The sort, filters and columns of a Dataset's Records, sent to the engine on Apply. */
export function RecordsControls({
  fields,
  query,
  onApply,
}: {
  readonly fields: readonly string[];
  readonly query: RecordsQuery;
  readonly onApply: (query: RecordsQuery) => void;
}): ReactNode {
  const { t } = useLingui();
  const id = useId();
  const apply = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const text = (name: string): string => {
      const value = form.get(name);
      return typeof value === "string" ? value : "";
    };
    const sortField = text("sort");
    const containsField = text("filter-field");
    const contains = text("contains");
    const wall = text("wall");
    const columns = form.getAll("column").filter((value) => typeof value === "string");
    onApply({
      ...(sortField === ""
        ? {}
        : {
            sort: {
              field: sortField,
              direction: text("order") === "descending" ? "descending" : "ascending",
            },
          }),
      ...(containsField === "" || contains === ""
        ? {}
        : { contains: { field: containsField, text: contains } }),
      ...(wall === "with" || wall === "without" ? { wall } : {}),
      ...(columns.length === fields.length ? {} : { columns }),
    });
  };
  const shown = query.columns ?? fields;
  return (
    <form aria-label={t`Records query`} onSubmit={apply} className="flex flex-wrap items-end gap-3">
      <label className="flex flex-col" htmlFor={`${id}-sort`}>
        <Trans>Sort by</Trans>
        <select
          id={`${id}-sort`}
          name="sort"
          defaultValue={query.sort?.field ?? ""}
          className="rounded-sm border px-1"
        >
          <option value="">{t`Number`}</option>
          {fields.map((field) => (
            <option key={field} value={field} translate="no">
              {field}
            </option>
          ))}
        </select>
      </label>
      <label className="flex flex-col" htmlFor={`${id}-order`}>
        <Trans>Order</Trans>
        <select
          id={`${id}-order`}
          name="order"
          defaultValue={query.sort?.direction ?? "ascending"}
          className="rounded-sm border px-1"
        >
          <option value="ascending">{t`Ascending`}</option>
          <option value="descending">{t`Descending`}</option>
        </select>
      </label>
      <label className="flex flex-col" htmlFor={`${id}-filter-field`}>
        <Trans>Filter on</Trans>
        <select
          id={`${id}-filter-field`}
          name="filter-field"
          defaultValue={query.contains?.field ?? ""}
          className="rounded-sm border px-1"
        >
          <option value="">{t`No field`}</option>
          {fields.map((field) => (
            <option key={field} value={field} translate="no">
              {field}
            </option>
          ))}
        </select>
      </label>
      <label className="flex flex-col" htmlFor={`${id}-contains`}>
        <Trans>Contains</Trans>
        <input
          id={`${id}-contains`}
          name="contains"
          type="text"
          dir="auto"
          defaultValue={query.contains?.text ?? ""}
          className="rounded-sm border px-1"
        />
      </label>
      <label className="flex flex-col" htmlFor={`${id}-wall`}>
        <Trans>Wall</Trans>
        <select
          id={`${id}-wall`}
          name="wall"
          defaultValue={query.wall ?? ""}
          className="rounded-sm border px-1"
        >
          <option value="">{t`Any`}</option>
          <option value="with">{t`Read past a Wall`}</option>
          <option value="without">{t`No Wall met`}</option>
        </select>
      </label>
      <fieldset className="flex flex-wrap gap-2">
        <legend>
          <Trans>Columns</Trans>
        </legend>
        {fields.map((field) => (
          <label key={field} className="flex items-center gap-1">
            <input
              type="checkbox"
              name="column"
              value={field}
              defaultChecked={shown.includes(field)}
            />
            <span translate="no">{field}</span>
          </label>
        ))}
      </fieldset>
      <button type="submit" className="rounded-sm border px-2 py-1">
        <Trans>Apply</Trans>
      </button>
    </form>
  );
}

/**
 * The page's offset of `element`'s top, in CSS pixels, kept as what is above
 * it changes height.
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

/** A row's height before it is measured, in CSS pixels. */
const ROW_ESTIMATE = 33;

/** The rows rendered beyond each edge of the window, so scrolling shows no gap. */
const OVERSCAN = 8;

/**
 * A Dataset's Records read so far, rendering only the rows in the window
 * and a few beyond (ADR 0071). The page scrolls, not the table; the table
 * names its rows for assistive technology, -1 while more are to read, and
 * asks for the next page once its last loaded rows come into view.
 */
export function RecordsGrid({
  dataset,
  columns,
  records,
  more,
  onNearEnd,
}: {
  readonly dataset: string;
  readonly columns: readonly string[];
  readonly records: readonly KeptRecord[];
  readonly more: boolean;
  readonly onNearEnd?: (() => void) | undefined;
}): ReactNode {
  const { t } = useLingui();
  const body = useRef<HTMLTableSectionElement>(null);
  const scrollMargin = useDocumentOffset(body);
  const rows = useWindowVirtualizer({
    count: records.length,
    estimateSize: () => ROW_ESTIMATE,
    overscan: OVERSCAN,
    scrollMargin,
    getItemKey: (index) => records[index]?.number ?? index,
  });
  const shown = rows.getVirtualItems();
  const last = shown.at(-1)?.index ?? -1;
  const nearEnd = more && last >= records.length - 1 - OVERSCAN;
  useEffect(() => {
    if (nearEnd) {
      onNearEnd?.();
    }
  }, [nearEnd, onNearEnd]);
  const before = (shown[0]?.start ?? scrollMargin) - scrollMargin;
  const after = rows.getTotalSize() - ((shown.at(-1)?.end ?? scrollMargin) - scrollMargin);
  return (
    <section aria-label={t`Records, scrollable`} tabIndex={0} className="overflow-x-auto">
      <table
        aria-label={t`Records of ${dataset}`}
        aria-rowcount={more ? -1 : records.length + 1}
        className="w-full border-collapse text-start"
      >
        <thead>
          <tr className="border-b">
            <th scope="col" className={cellClassName}>
              <Trans>Number</Trans>
            </th>
            {columns.map((column) => (
              <th key={column} scope="col" className={cellClassName} dir="auto" translate="no">
                {column}
              </th>
            ))}
          </tr>
        </thead>
        <tbody ref={body}>
          {before > 0 ? <tr aria-hidden="true" style={{ height: before }} /> : null}
          {shown.map((item) => {
            const record = records[item.index];
            if (record === undefined) {
              return null;
            }
            const value: Readonly<Record<string, unknown>> =
              typeof record.value === "object" && record.value !== null ? record.value : {};
            return (
              <tr
                key={item.key}
                data-index={item.index}
                ref={rows.measureElement}
                aria-rowindex={item.index + 2}
              >
                <th scope="row" className={cellClassName}>
                  {record.number}
                </th>
                {columns.map((column) => (
                  <td key={column} className={cellClassName} dir="auto" translate="no">
                    {cellText(value[column])}
                  </td>
                ))}
              </tr>
            );
          })}
          {after > 0 ? <tr aria-hidden="true" style={{ height: after }} /> : null}
        </tbody>
      </table>
    </section>
  );
}

/** What the grid read of a Dataset under one query. */
type Read =
  | { readonly kind: "reading" }
  | { readonly kind: "refused"; readonly error: unknown }
  | {
      readonly kind: "read";
      readonly records: readonly DatasetRecord[];
      readonly through: number;
      readonly next: number | undefined;
    };

/** A Dataset's Records under `query`, read a page at a time, and the reader of the next page. */
function useRecords(
  { client, runId }: RunViewProps,
  dataset: string,
  query: RecordsQuery,
): { readonly read: Read; readonly readMore: () => void } {
  const [read, setRead] = useState<Read>({ kind: "reading" });
  const reading = useRef<AbortController | null>(null);
  useEffect(() => {
    const abort = new AbortController();
    reading.current = abort;
    setRead({ kind: "reading" });
    const ask = async (): Promise<void> => {
      try {
        const page = await callFoxhound(
          client,
          "dataset.page",
          pageParams(runId, dataset, query),
          abort.signal,
        );
        if (!abort.signal.aborted) {
          reading.current = null;
          setRead({
            kind: "read",
            records: page.records,
            through: page.through,
            next: page.next ?? undefined,
          });
        }
      } catch (error) {
        if (!abort.signal.aborted) {
          reading.current = null;
          setRead({ kind: "refused", error });
        }
      }
    };
    void ask();
    return () => abort.abort();
  }, [client, runId, dataset, query]);
  const readMore = (): void => {
    if (read.kind !== "read" || read.next === undefined || reading.current !== null) {
      return;
    }
    const abort = new AbortController();
    reading.current = abort;
    const after = { after: read.next, through: read.through };
    const ask = async (): Promise<void> => {
      try {
        const page = await callFoxhound(
          client,
          "dataset.page",
          pageParams(runId, dataset, query, after),
          abort.signal,
        );
        reading.current = null;
        setRead((current) =>
          current.kind === "read" && current.next === after.after
            ? {
                kind: "read",
                records: [...current.records, ...page.records],
                through: current.through,
                next: page.next ?? undefined,
              }
            : current,
        );
      } catch (error) {
        reading.current = null;
        setRead({ kind: "refused", error });
      }
    };
    void ask();
  };
  return { read, readMore };
}

/** One Dataset's Records, with the controls that ask the engine for them. */
function Records({
  props,
  dataset,
}: {
  readonly props: RunViewProps;
  readonly dataset: DatasetSummary;
}): ReactNode {
  const [query, setQuery] = useState<RecordsQuery>({});
  const { read, readMore } = useRecords(props, dataset.name, query);
  const records = read.kind === "read" ? read.records : [];
  const fields = fieldsOf(dataset, records);
  const columns = query.columns ?? fields;
  const { offline, runId } = props;
  // How many Records the browser keeps of the view, once kept offline.
  const [kept, setKept] = useState<number | undefined>(undefined);
  const keep = async (): Promise<void> => {
    if (offline === undefined) {
      return;
    }
    const name = keptDatasetOf(runId, dataset.name);
    await offline.put("dataset_view", name, keptView(runId, dataset, columns, records), name);
    setKept(records.length);
  };
  return (
    <div className="flex flex-col gap-3">
      <RecordsControls key={dataset.name} fields={fields} query={query} onApply={setQuery} />
      {offline !== undefined && read.kind === "read" ? (
        <div className="flex flex-wrap items-center gap-3">
          <button type="button" onClick={() => void keep()} className="rounded-sm border px-2 py-1">
            <Trans>Keep offline</Trans>
          </button>
          {kept === undefined ? null : (
            <output>
              <Plural
                value={kept}
                one="Kept offline in this browser: # Record, readable until you sign out"
                other="Kept offline in this browser: # Records, readable until you sign out"
              />
            </output>
          )}
        </div>
      ) : null}
      {read.kind === "refused" ? <ErrorMessage error={read.error} /> : null}
      {read.kind === "reading" ? (
        <output className="block">
          <Trans>Asking the engine…</Trans>
        </output>
      ) : null}
      {read.kind === "read" ? (
        <>
          <output className="block">
            <Plural value={records.length} one="# Record read" other="# Records read" />
          </output>
          <RecordsGrid
            dataset={dataset.name}
            columns={columns}
            records={records}
            more={read.next !== undefined}
            onNearEnd={readMore}
          />
        </>
      ) : null}
    </div>
  );
}

/** What the view read of the crawl's Datasets. */
type Listed =
  | { readonly kind: "reading" }
  | { readonly kind: "refused"; readonly error: unknown }
  | { readonly kind: "listed"; readonly datasets: readonly DatasetSummary[] };

/** The views of the run the offline cache keeps, read with no Connection while the engine has not answered (`reading`). */
function useKeptViews(
  { client, runId, offline }: RunViewProps,
  reading: boolean,
): readonly KeptView[] {
  // Whether the page has a cache and no Connection: without a cache it
  // never changes, so a page with none renders nothing more.
  const disconnected = useSyncExternalStore(
    (changed) => client.watch(changed),
    () => offline !== undefined && client.state !== "connected",
  );
  const wanted = reading && disconnected;
  const [kept, setKept] = useState<readonly KeptView[]>([]);
  useEffect(() => {
    if (!wanted || offline === undefined) {
      return undefined;
    }
    let current = true;
    const read = async (): Promise<void> => {
      const views = (await offline.all("dataset_view")).filter((view) => isKeptViewOf(view, runId));
      if (current) {
        setKept(views);
      }
    };
    void read();
    return () => {
      current = false;
    };
  }, [wanted, offline, runId]);
  return wanted ? kept : [];
}

/** The views of a run the browser keeps offline, read-only, the one chosen in its grid. */
export function KeptViews({ views }: { readonly views: readonly KeptView[] }): ReactNode {
  const [chosen, setChosen] = useState<string | undefined>(undefined);
  const shown = views.find((view) => view.dataset.name === chosen) ?? views[0];
  return (
    <div className="flex flex-col gap-4">
      <p>
        <Trans>
          This browser's offline copy of the Dataset views kept offline, as they were kept: the
          engine is not reachable.
        </Trans>
      </p>
      <DatasetList datasets={views.map((view) => view.dataset)} />
      {shown === undefined ? null : (
        <>
          <label className="flex flex-col">
            <Trans>Records of the Dataset</Trans>
            <select
              value={shown.dataset.name}
              onChange={(event) => setChosen(event.target.value)}
              className="rounded-sm border px-1"
            >
              {views.map((view) => (
                <option key={view.dataset.name} value={view.dataset.name} translate="no">
                  {view.dataset.name}
                </option>
              ))}
            </select>
          </label>
          <RecordsGrid
            key={shown.dataset.name}
            dataset={shown.dataset.name}
            columns={shown.columns}
            records={shown.records}
            more={false}
          />
        </>
      )}
    </div>
  );
}

/** A crawl run's Datasets, and the Records of the one chosen, the first at the start. */
export function Datasets(props: RunViewProps): ReactNode {
  const { client, runId } = props;
  const [listed, setListed] = useState<Listed>({ kind: "reading" });
  const [chosen, setChosen] = useState<string | undefined>(undefined);
  useEffect(() => {
    const abort = new AbortController();
    const ask = async (): Promise<void> => {
      try {
        const { datasets } = await callFoxhound(
          client,
          "dataset.list",
          { run_id: runId },
          abort.signal,
        );
        if (!abort.signal.aborted) {
          setListed({ kind: "listed", datasets });
        }
      } catch (error) {
        if (!abort.signal.aborted) {
          setListed({ kind: "refused", error });
        }
      }
    };
    void ask();
    return () => abort.abort();
  }, [client, runId]);
  const kept = useKeptViews(props, listed.kind === "reading");
  if (listed.kind === "refused") {
    return <ErrorMessage error={listed.error} />;
  }
  if (listed.kind === "reading" && kept.length > 0) {
    return <KeptViews views={kept} />;
  }
  if (listed.kind === "reading") {
    return (
      <output className="block">
        <Trans>Asking the engine…</Trans>
      </output>
    );
  }
  const { datasets } = listed;
  if (datasets.length === 0) {
    return (
      <p>
        <Trans>The crawl declares no Dataset.</Trans>
      </p>
    );
  }
  const shown = datasets.find((dataset) => dataset.name === chosen) ?? datasets[0];
  return (
    <div className="flex flex-col gap-4">
      <DatasetList datasets={datasets} />
      {shown === undefined ? null : (
        <>
          <label className="flex flex-col">
            <Trans>Records of the Dataset</Trans>
            <select
              value={shown.name}
              onChange={(event) => setChosen(event.target.value)}
              className="rounded-sm border px-1"
            >
              {datasets.map((dataset) => (
                <option key={dataset.name} value={dataset.name} translate="no">
                  {dataset.name}
                </option>
              ))}
            </select>
          </label>
          <Records key={shown.name} props={props} dataset={shown} />
        </>
      )}
    </div>
  );
}
