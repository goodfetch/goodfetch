// A crawl run's request log (spec #593, story 219; ADRs 0071, 0079, 0102,
// 0104, 0109 and 0117): the outcome of each crawl request, followed live
// with `run.outcomes`, which reads the run's committed state and waits for
// the next commit, so a reader never slows the crawl. Its filters (the
// outcome, the Label and the Error code) run in the engine: applying them
// starts the read again with them. Each request shows its attempts and the
// error of each failed attempt; a `handler_failed` or `hook_failed` shows
// the program's exception, its type, message and stack (a Rust program's
// type reads `BoxError` or `panic`, with no stack), and a `hook_failed`
// its stage, Hook, Label and wrapped code. A request `run.retry_failed`
// re-admitted shows its later outcome only. The log renders only the rows
// on screen and a few beyond, so a heavy log keeps the page light.
import type { RunViewProps } from "@goodfetch/console/console-build";
import { ErrorMessage } from "@goodfetch/console/error-message";
import { formatInstant } from "@goodfetch/console/format";
import type {
  AttemptError,
  CrawlRequestOutcome,
  OutcomeKind,
  RunOutcomesParams,
} from "@goodfetch/foxhound/generated";
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import { Plural, Trans, useLingui } from "@lingui/react/macro";
import { useWindowVirtualizer } from "@tanstack/react-virtual";
import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
  type SubmitEvent,
} from "react";
import { callFoxhound } from "#call.ts";

/** What the person asked the engine to keep; an empty list keeps every value. */
export interface LogFilters {
  readonly kinds: readonly OutcomeKind[];
  readonly labels: readonly string[];
  readonly codes: readonly string[];
}

const NO_FILTERS: LogFilters = { kinds: [], labels: [], codes: [] };

/** The params of the read after `after` (from the first when absent) under `filters`. */
export function outcomesParams(
  runId: string,
  filters: LogFilters,
  after?: number,
): RunOutcomesParams {
  return {
    run_id: runId,
    ...(after === undefined ? {} : { after }),
    ...(filters.kinds.length === 0 ? {} : { kinds: [...filters.kinds] }),
    ...(filters.labels.length === 0 ? {} : { labels: [...filters.labels] }),
    ...(filters.codes.length === 0 ? {} : { codes: [...filters.codes] }),
  };
}

/** What the log read so far. */
type Followed =
  | { readonly kind: "reading" }
  | { readonly kind: "refused"; readonly error: unknown }
  | {
      readonly kind: "following";
      readonly outcomes: readonly CrawlRequestOutcome[];
      readonly ended: boolean;
    };

/**
 * The outcomes `outcomes` and `page` hold together, each request's latest
 * only: a later outcome of a request supersedes its earlier one (ADR 0102).
 */
function merged(
  outcomes: readonly CrawlRequestOutcome[],
  page: readonly CrawlRequestOutcome[],
): readonly CrawlRequestOutcome[] {
  const latest = new Map<string, CrawlRequestOutcome>();
  for (const outcome of [...outcomes, ...page]) {
    // Deleting first moves a superseded request to its later place in commit order.
    latest.delete(outcome.request.id);
    latest.set(outcome.request.id, outcome);
  }
  return [...latest.values()];
}

/** Follows the run `runId`'s outcomes that `filters` keep, page by page, until the sequence ends. */
function useOutcomes(client: RunViewProps["client"], runId: string, filters: LogFilters): Followed {
  const [followed, setFollowed] = useState<Followed>({ kind: "reading" });
  useEffect(() => {
    const abort = new AbortController();
    const { signal } = abort;
    setFollowed({ kind: "reading" });
    const follow = async (): Promise<void> => {
      try {
        let outcomes: readonly CrawlRequestOutcome[] = [];
        let after: number | undefined;
        for (;;) {
          const page = await callFoxhound(
            client,
            "run.outcomes",
            outcomesParams(runId, filters, after),
            signal,
          );
          signal.throwIfAborted();
          outcomes = merged(outcomes, page.outcomes);
          setFollowed({ kind: "following", outcomes, ended: page.ended });
          if (page.ended) {
            return;
          }
          after = page.next;
        }
      } catch (error) {
        if (!signal.aborted) {
          setFollowed({ kind: "refused", error });
        }
      }
    };
    void follow();
    return () => abort.abort();
  }, [client, runId, filters]);
  return followed;
}

/** The name of each kind of outcome. */
const OUTCOME_NAMES: Readonly<Record<OutcomeKind, MessageDescriptor>> = {
  succeeded: msg`Succeeded`,
  failed: msg`Failed`,
  skipped: msg`Skipped`,
  blocked: msg`Blocked`,
};

const KINDS: readonly OutcomeKind[] = ["succeeded", "failed", "skipped", "blocked"];

/** `text` split at commas, each value trimmed, the empty ones left out. */
function valuesOf(text: string): readonly string[] {
  return text
    .split(",")
    .map((value) => value.trim())
    .filter((value) => value !== "");
}

/** The log's filters, sent to the engine on Apply. */
export function LogControls({
  filters,
  onApply,
}: {
  readonly filters: LogFilters;
  readonly onApply: (filters: LogFilters) => void;
}): ReactNode {
  const { t, i18n } = useLingui();
  const id = useId();
  const apply = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const form = new FormData(event.currentTarget);
    const text = (name: string): string => {
      const value = form.get(name);
      return typeof value === "string" ? value : "";
    };
    const checked = new Set(form.getAll("kind"));
    onApply({
      kinds: KINDS.filter((kind) => checked.has(kind)),
      labels: valuesOf(text("labels")),
      codes: valuesOf(text("codes")),
    });
  };
  return (
    <form aria-label={t`Log filters`} onSubmit={apply} className="flex flex-wrap items-end gap-3">
      <fieldset className="flex flex-wrap gap-2">
        <legend>
          <Trans>Outcome</Trans>
        </legend>
        {KINDS.map((kind) => (
          <label key={kind} className="flex items-center gap-1">
            <input
              type="checkbox"
              name="kind"
              value={kind}
              defaultChecked={filters.kinds.includes(kind)}
            />
            {i18n._(OUTCOME_NAMES[kind])}
          </label>
        ))}
      </fieldset>
      <label className="flex flex-col" htmlFor={`${id}-labels`}>
        <Trans>Labels, separated by commas</Trans>
        <input
          id={`${id}-labels`}
          name="labels"
          type="text"
          dir="auto"
          translate="no"
          defaultValue={filters.labels.join(", ")}
          className="rounded-sm border px-1"
        />
      </label>
      <label className="flex flex-col" htmlFor={`${id}-codes`}>
        <Trans>Error codes, separated by commas</Trans>
        <input
          id={`${id}-codes`}
          name="codes"
          type="text"
          dir="ltr"
          translate="no"
          defaultValue={filters.codes.join(", ")}
          className="rounded-sm border px-1"
        />
      </label>
      <button type="submit" className="rounded-sm border px-2 py-1">
        <Trans>Apply</Trans>
      </button>
    </form>
  );
}

/** The Error codes whose details name the program's own exception (ADRs 0079 and 0104). */
const PROGRAM_EXCEPTION_CODES: ReadonlySet<string> = new Set(["handler_failed", "hook_failed"]);

/** `details[name]` when it is text. */
function textOf(details: AttemptError["details"], name: string): string | undefined {
  const value = details?.[name];
  return typeof value === "string" ? value : undefined;
}

const termClassName = "font-semibold";

/** The program's exception an attempt failed with: its type, message and stack. */
function ProgramException({ details }: { readonly details: AttemptError["details"] }): ReactNode {
  const type = textOf(details, "exception_type");
  const message = textOf(details, "exception_message");
  const stack = textOf(details, "exception_stack") ?? "";
  if (type === undefined && message === undefined) {
    return null;
  }
  return (
    <>
      <dt className={termClassName}>
        <Trans>Exception type</Trans>
      </dt>
      <dd translate="no">
        <code dir="ltr">{type}</code>
      </dd>
      <dt className={termClassName}>
        <Trans>Exception message</Trans>
      </dt>
      <dd dir="auto" translate="no">
        {message}
      </dd>
      <dt className={termClassName}>
        <Trans>Stack</Trans>
      </dt>
      <dd>
        {stack === "" ? (
          <Trans>No stack</Trans>
        ) : (
          <pre dir="ltr" translate="no" className="overflow-x-auto text-sm whitespace-pre-wrap">
            {stack}
          </pre>
        )}
      </dd>
    </>
  );
}

/** The Hook a `hook_failed` names: its stage, its name, the request's Label and the wrapped code. */
function FailedHook({ details }: { readonly details: AttemptError["details"] }): ReactNode {
  const label = textOf(details, "label");
  const wrapped = textOf(details, "wrapped_code");
  return (
    <>
      <dt className={termClassName}>
        <Trans>Stage</Trans>
      </dt>
      <dd translate="no">{textOf(details, "stage")}</dd>
      <dt className={termClassName}>
        <Trans>Hook</Trans>
      </dt>
      <dd dir="auto" translate="no">
        {textOf(details, "hook")}
      </dd>
      <dt className={termClassName}>
        <Trans>Label</Trans>
      </dt>
      <dd translate="no">{label ?? <Trans>None: a lifecycle stage has no request</Trans>}</dd>
      <dt className={termClassName}>
        <Trans>Wrapped code</Trans>
      </dt>
      <dd translate="no">
        {wrapped === undefined ? (
          <Trans>None: the program's own exception</Trans>
        ) : (
          <code dir="ltr">{wrapped}</code>
        )}
      </dd>
    </>
  );
}

/** The error of one failed attempt. */
function AttemptFailure({ error }: { readonly error: AttemptError }): ReactNode {
  const { t } = useLingui();
  const attempt = error.attempt;
  return (
    <li>
      <dl aria-label={t`Attempt ${attempt}`} className="grid grid-cols-[auto_1fr] gap-x-2">
        <dt className={termClassName}>
          <Trans>Attempt</Trans>
        </dt>
        <dd>{attempt}</dd>
        <dt className={termClassName}>
          <Trans>Error code</Trans>
        </dt>
        <dd translate="no">
          <code dir="ltr">{error.code}</code>
        </dd>
        <dt className={termClassName}>
          <Trans>Message</Trans>
        </dt>
        <dd dir="auto" translate="no">
          {error.message}
        </dd>
        {error.code === "hook_failed" ? <FailedHook details={error.details} /> : null}
        {PROGRAM_EXCEPTION_CODES.has(error.code) ? (
          <ProgramException details={error.details} />
        ) : null}
      </dl>
    </li>
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

const cellClassName = "border-b px-2 py-1 text-start align-top";

/** The outcomes read so far, rendering only the rows in the window and a few beyond (ADR 0071). */
export function OutcomeLog({
  outcomes,
}: {
  readonly outcomes: readonly CrawlRequestOutcome[];
}): ReactNode {
  const { t, i18n } = useLingui();
  const body = useRef<HTMLTableSectionElement>(null);
  const scrollMargin = useDocumentOffset(body);
  const rows = useWindowVirtualizer({
    count: outcomes.length,
    estimateSize: () => ROW_ESTIMATE,
    overscan: OVERSCAN,
    scrollMargin,
    getItemKey: (index) => outcomes[index]?.request.id ?? index,
  });
  const shown = rows.getVirtualItems();
  const before = (shown[0]?.start ?? scrollMargin) - scrollMargin;
  const after = rows.getTotalSize() - ((shown.at(-1)?.end ?? scrollMargin) - scrollMargin);
  return (
    <section aria-label={t`Request log, scrollable`} tabIndex={0} className="overflow-x-auto">
      <table
        aria-label={t`Request log`}
        aria-rowcount={outcomes.length + 1}
        className="w-full border-collapse text-start"
      >
        <thead>
          <tr className="border-b">
            <th scope="col" className={cellClassName}>
              <Trans>URL</Trans>
            </th>
            <th scope="col" className={cellClassName}>
              <Trans>Label</Trans>
            </th>
            <th scope="col" className={cellClassName}>
              <Trans>Outcome</Trans>
            </th>
            <th scope="col" className={cellClassName}>
              <Trans>Attempts</Trans>
            </th>
            <th scope="col" className={cellClassName}>
              <Trans>Ended</Trans>
            </th>
            <th scope="col" className={cellClassName}>
              <Trans>Errors of the failed attempts</Trans>
            </th>
          </tr>
        </thead>
        <tbody ref={body}>
          {before > 0 ? <tr aria-hidden="true" style={{ height: before }} /> : null}
          {shown.map((item) => {
            const outcome = outcomes[item.index];
            if (outcome === undefined) {
              return null;
            }
            return (
              <tr
                key={item.key}
                data-index={item.index}
                ref={rows.measureElement}
                aria-rowindex={item.index + 2}
              >
                <th scope="row" className={`${cellClassName} break-all`} translate="no">
                  {outcome.request.url}
                </th>
                <td className={cellClassName} translate="no">
                  {outcome.request.label}
                </td>
                <td className={cellClassName}>{i18n._(OUTCOME_NAMES[outcome.kind])}</td>
                <td className={cellClassName}>{outcome.attempts}</td>
                <td className={cellClassName}>
                  <time dateTime={outcome.ended_at} translate="no">
                    {formatInstant(outcome.ended_at)}
                  </time>
                </td>
                <td className={cellClassName}>
                  {outcome.errors.length === 0 ? (
                    <Trans>None</Trans>
                  ) : (
                    <ol className="flex flex-col gap-2">
                      {outcome.errors.map((error) => (
                        <AttemptFailure key={error.attempt} error={error} />
                      ))}
                    </ol>
                  )}
                </td>
              </tr>
            );
          })}
          {after > 0 ? <tr aria-hidden="true" style={{ height: after }} /> : null}
        </tbody>
      </table>
    </section>
  );
}

/** A crawl run's request log, followed live, with its filters. */
export function RequestLog({ client, runId }: RunViewProps): ReactNode {
  const [filters, setFilters] = useState<LogFilters>(NO_FILTERS);
  const followed = useOutcomes(client, runId, filters);
  return (
    <div className="flex flex-col gap-3">
      <LogControls filters={filters} onApply={setFilters} />
      {followed.kind === "refused" ? <ErrorMessage error={followed.error} /> : null}
      {followed.kind === "reading" ? (
        <output className="block">
          <Trans>Waiting for the first outcome the crawl commits…</Trans>
        </output>
      ) : null}
      {followed.kind === "following" ? (
        <>
          <output className="block">
            <Plural value={followed.outcomes.length} one="# outcome" other="# outcomes" />{" "}
            {followed.ended ? (
              <Trans>The crawl ended.</Trans>
            ) : (
              <Trans>Following the crawl live.</Trans>
            )}
          </output>
          <OutcomeLog outcomes={followed.outcomes} />
        </>
      ) : null}
    </div>
  );
}
