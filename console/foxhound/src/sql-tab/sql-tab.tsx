// The SQL tab (spec #593, stories 210 and 211; ADRs 0031 and 0057): one
// read-only statement over the runs a person selects, this run first, at
// most 100, as `dataset.query` runs it. Each run's tables are functions
// that take its Run ID (`records('run')`), which the tab's first statement
// shows. The result shows its columns with their types and one page of
// rows at a time, so its DOM stays bounded whatever the engine returned,
// and how many rows the query produced beyond those returned. A refusal
// shows as the Console shows any, its fix included.
import type { RunView, RunViewProps } from "@goodfetch/console/console-build";
import { ErrorMessage } from "@goodfetch/console/error-message";
import { Form } from "@goodfetch/console/form";
import type { DatasetQueryResult } from "@goodfetch/foxhound/generated";
import { msg } from "@lingui/core/macro";
import { Trans, useLingui } from "@lingui/react/macro";
import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import { callFoxhound } from "#call.ts";

/** The most runs one query selects. */
export const MAX_RUNS = 100;

/** The rows one page of the result shows. */
export const PAGE_ROWS = 100;

/** What the tab's last statement came to. */
export type Ran =
  | { readonly kind: "idle" }
  | { readonly kind: "running" }
  | { readonly kind: "refused"; readonly error: unknown }
  | { readonly kind: "answered"; readonly result: DatasetQueryResult };

/** The Run IDs `text` names, separated by spaces, commas or lines, each once, in order. */
export function runsOf(text: string): string[] {
  return [...new Set(text.split(/[\s,]+/u).filter((run) => run.length > 0))];
}

/** A cell's value as text: a string as itself, any other value as JSON. */
function cellText(value: unknown): string {
  if (value === null) {
    return "NULL";
  }
  return typeof value === "string" ? value : JSON.stringify(value);
}

const buttonClassName =
  "rounded-md border px-2 py-1 text-sm font-medium focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50";
const primaryClassName =
  "rounded-md bg-primary px-2 py-1 text-sm font-medium text-primary-foreground focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50";
const fieldClassName = "rounded-md border bg-background px-2 py-1 font-mono text-sm";
const cellClassName = "border-b px-2 py-1 text-start align-top";

/** One page of a query's result: its columns and the rows of page `page`. */
function ResultTable({
  result,
  page,
  onPage,
}: {
  readonly result: DatasetQueryResult;
  readonly page: number;
  readonly onPage: (page: number) => void;
}): ReactNode {
  const { t } = useLingui();
  const returned = result.rows.length;
  const pages = Math.max(1, Math.ceil(returned / PAGE_ROWS));
  const first = page * PAGE_ROWS;
  const shown = result.rows.slice(first, first + PAGE_ROWS);
  const from = returned === 0 ? 0 : first + 1;
  const to = first + shown.length;
  const total = result.total;
  return (
    <div className="flex flex-col gap-2">
      <p>
        {total > returned ? (
          <Trans>
            Rows {from} to {to} of {returned} returned; the query produced {total}.
          </Trans>
        ) : (
          <Trans>
            Rows {from} to {to} of {returned}.
          </Trans>
        )}
      </p>
      <div className="overflow-x-auto">
        <table className="w-full text-start">
          <thead>
            <tr>
              {result.columns.map((column, index) => (
                // Two columns may share a name; their place is their identity.
                <th key={index} scope="col" className={cellClassName}>
                  <span translate="no" dir="auto">
                    {column.name}
                  </span>{" "}
                  <span className="font-normal text-muted-foreground" translate="no">
                    {column.type}
                  </span>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {shown.map((row, index) => (
              // A row's place in the result is its identity: the result has no key.
              <tr key={first + index}>
                {row.map((value, column) => (
                  <td
                    key={column}
                    className={`${cellClassName} break-all font-mono text-sm`}
                    translate="no"
                    dir="auto"
                  >
                    {cellText(value)}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {pages > 1 ? (
        <nav className="flex items-center gap-2" aria-label={t`Result pages`}>
          <button
            type="button"
            className={buttonClassName}
            disabled={page === 0}
            onClick={() => onPage(page - 1)}
          >
            <Trans>Previous rows</Trans>
          </button>
          <button
            type="button"
            className={buttonClassName}
            disabled={page + 1 >= pages}
            onClick={() => onPage(page + 1)}
          >
            <Trans>Next rows</Trans>
          </button>
        </nav>
      ) : null}
    </div>
  );
}

/** What the SQL tab shows: its form, and what its last statement came to. */
export interface SqlTabViewProps {
  /** The run whose page shows the tab, which its first statement reads. */
  readonly runId: string;
  /** What the last statement came to. */
  readonly ran: Ran;
  /** The page of the result shown, from 0. */
  readonly page: number;
  /** Shows another page of the result. */
  readonly onPage: (page: number) => void;
  /** Runs `sql` over `runs`. */
  readonly onRun: (sql: string, runs: string[]) => void;
}

/** The SQL tab as it shows `ran`. */
export function SqlTabView({ runId, ran, page, onPage, onRun }: SqlTabViewProps): ReactNode {
  const statementId = useId();
  const runsId = useId();
  return (
    <div className="flex flex-col gap-4">
      <Form
        className="flex flex-col gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          const form = new FormData(event.currentTarget);
          const sql = form.get("sql");
          const runs = form.get("runs");
          onRun(
            typeof sql === "string" ? sql : "",
            runsOf(typeof runs === "string" ? runs : runId).slice(0, MAX_RUNS),
          );
        }}
      >
        <label htmlFor={statementId} className="font-semibold">
          <Trans>Statement</Trans>
        </label>
        <textarea
          id={statementId}
          name="sql"
          rows={6}
          spellCheck={false}
          translate="no"
          defaultValue={`SELECT number, value FROM records('${runId}') ORDER BY number`}
          className={fieldClassName}
        />
        <label htmlFor={runsId} className="font-semibold">
          <Trans>Runs it reads</Trans>
        </label>
        <p className="text-sm text-muted-foreground" id={`${runsId}-hint`}>
          <Trans>
            Run IDs separated by spaces, at most {MAX_RUNS}. Only these runs are readable, each
            through its table functions: records, rejected_records and datasets.
          </Trans>
        </p>
        <input
          id={runsId}
          name="runs"
          translate="no"
          defaultValue={runId}
          aria-describedby={`${runsId}-hint`}
          className={fieldClassName}
        />
        <div>
          <button type="submit" disabled={ran.kind === "running"} className={primaryClassName}>
            <Trans>Run the statement</Trans>
          </button>
        </div>
      </Form>
      <section aria-live="polite" aria-busy={ran.kind === "running"}>
        {ran.kind === "running" ? (
          <output className="block">
            <Trans>Asking the engine…</Trans>
          </output>
        ) : null}
        {ran.kind === "refused" ? <ErrorMessage error={ran.error} /> : null}
        {ran.kind === "answered" ? (
          <ResultTable result={ran.result} page={page} onPage={onPage} />
        ) : null}
      </section>
    </div>
  );
}

/** The SQL tab of the run `runId`, running each statement through `dataset.query`. */
export function SqlTab({ client, runId }: RunViewProps): ReactNode {
  const [ran, setRan] = useState<Ran>({ kind: "idle" });
  const [page, setPage] = useState(0);
  const running = useRef<AbortController | null>(null);
  useEffect(() => () => running.current?.abort(), []);

  const run = async (sql: string, runs: string[]): Promise<void> => {
    running.current?.abort();
    const abort = new AbortController();
    running.current = abort;
    setRan({ kind: "running" });
    setPage(0);
    try {
      // The form always selects this run when it names none.
      const [first = runId, ...rest] = runs;
      const result = await callFoxhound(
        client,
        "dataset.query",
        { sql, runs: [first, ...rest] },
        abort.signal,
      );
      if (!abort.signal.aborted) {
        setRan({ kind: "answered", result });
      }
    } catch (error) {
      if (!abort.signal.aborted) {
        setRan({ kind: "refused", error });
      }
    }
  };

  return (
    <SqlTabView
      runId={runId}
      ran={ran}
      page={page}
      onPage={setPage}
      onRun={(sql, runs) => void run(sql, runs)}
    />
  );
}

/** The SQL tab, as a view of a run that needs Foxhound. */
export const SQL_TAB_VIEW: RunView = {
  id: "sql",
  name: msg`SQL`,
  content: (props) => <SqlTab {...props} />,
};
