// A run's page (spec #591, stories 11, 14, 16 and 18 to 20; ADRs 0029,
// 0031, 0041, 0102, 0107 and 0122), at the address a Run report's
// `consoleUrl` gives, `/runs/<run-id>`. It shows the run's Lifecycle,
// Verdict and Completeness as three values, marks it *uncapped* when it can
// spend money with nothing capping it, whatever its Model allowance, and
// shows its writer, run epoch, last Checkpoint, each Human pause waiting
// and the worst storage level among the filesystems it writes to, with
// that folder and its free space. A live run offers stop and checkpoint; an
// interrupted or stopped run shows the command that resumes it, with
// **Copy resume command**.
//
// The page follows the run through `run.progress`, each read after the
// position it last read, reading the run's Usage with each value and its
// Run index row again once its Lifecycle changes, until the value that
// ends the sequence. An axis change of the run or a resync on the event
// stream reads it all again once it stopped following, as when an engine
// resumes a run no engine worked.
import type {
  Completeness,
  HumanPause,
  Product,
  RunIndexRow,
  RunProgress,
  StorageLevel,
  Verdict,
} from "@goodfetch/collie-client/generated";
import { GoodfetchError } from "@goodfetch/collie-client/generated";
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import { Trans, useLingui } from "@lingui/react/macro";
import { useEffect, useReducer, useState, useSyncExternalStore, type ReactNode } from "react";
import type { ConsoleClient } from "#client/client.ts";
import { formatBytes, formatCount, formatInstant } from "#format.ts";
import { ErrorMessage } from "#errors/error-message.tsx";
import { RUNS } from "#routes.ts";
import { COMPLETENESS_NAMES, LIFECYCLE_NAMES, VERDICT_NAMES } from "./names.ts";

/** A run's three axes (ADR 0029): its Verdict and Completeness once it ended. */
export interface RunAxisValues {
  readonly lifecycle: RunProgress["lifecycle"];
  readonly verdict?: Verdict | null | undefined;
  readonly completeness?: Completeness | null | undefined;
}

/** What the page shows of a run it read. */
export interface ShownRun {
  readonly progress: RunProgress;
  readonly axes: RunAxisValues;
  /** The Product the run needs, whose executable resumes it. */
  readonly needs: Product;
  /** Whether it spent, or reserved, money no Limit caps (ADR 0041). */
  readonly uncapped: boolean;
}

/** What the page read of its run. */
export type RunRead =
  | { readonly kind: "reading" }
  /** The Run index lists no run with that Run ID (`run_not_found`). */
  | { readonly kind: "not_listed" }
  /** The engine refused; its English message, which the code's catalog message replaces once #912 maps it (ADR 0079). */
  | { readonly kind: "failed"; readonly error: unknown }
  | { readonly kind: "shown"; readonly run: ShownRun };

/** What the last action a person took on the page came to. */
export type Notice =
  | { readonly kind: "checkpoint_written"; readonly at: string }
  | { readonly kind: "stopped" }
  | { readonly kind: "copied" }
  | { readonly kind: "copy_refused" }
  | { readonly kind: "refused"; readonly error: unknown };

/** What a run's page shows. */
export interface RunPageViewProps {
  readonly runId: string;
  readonly read: RunRead;
  /** Whether the Console has no Connection now, which disables the page's actions. */
  readonly readOnly?: boolean | undefined;
  /** Whether an action the page asked for is under way. */
  readonly busy?: boolean | undefined;
  readonly notice?: Notice | undefined;
  readonly onStop?: (() => void) | undefined;
  readonly onCheckpoint?: (() => void) | undefined;
  readonly onCopy?: ((command: string) => void) | undefined;
}

/** Each Product's executable, whose `run resume` resumes a run that needs it (ADR 0090). */
const EXECUTABLES: Readonly<Record<Product, string>> = {
  collie: "goodfetch-collie",
  foxhound: "goodfetch-foxhound",
};

/** The command that resumes `runId`, by its Run ID, with the executable of the Product it needs. */
export function resumeCommand(runId: string, needs: Product): string {
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- a command line, which no language translates.
  return `${EXECUTABLES[needs]} run resume ${runId}`;
}

const STORAGE_LEVEL_NAMES: Readonly<Record<StorageLevel, MessageDescriptor>> = {
  normal: msg`normal`,
  low: msg`low`,
  critical: msg`critical`,
};

/** Whether no engine works `run` though it has not ended: it was interrupted, as by a crash. */
function interrupted({ progress }: ShownRun): boolean {
  return progress.lifecycle === "running" && progress.writer === "none";
}

/** Whether a start with its Run ID would take `run` on again: it was interrupted or stopped. */
function resumable(run: ShownRun): boolean {
  return interrupted(run) || run.progress.lifecycle === "stopped";
}

/** Whether `run` is live here: an engine works it and it has not ended. */
function live({ progress }: ShownRun): boolean {
  return progress.lifecycle === "running" && progress.writer === "live";
}

/** A point in time, with its machine form beside it. */
function Instant({ value }: { readonly value: string }): ReactNode {
  return (
    <time dateTime={value} translate="no">
      {formatInstant(value)}
    </time>
  );
}

const sectionClassName = "flex flex-col gap-3 rounded-lg border p-4";
const listClassName = "grid grid-cols-[max-content_1fr] gap-x-6 gap-y-2";
const buttonClassName =
  "rounded-md border px-3 py-1.5 font-medium focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50";

/** One term of a description list and its value. */
function Fact({
  term,
  children,
}: {
  readonly term: ReactNode;
  readonly children: ReactNode;
}): ReactNode {
  return (
    <>
      <dt className="font-medium">{term}</dt>
      <dd>{children}</dd>
    </>
  );
}

/** The run's three axes, each its own value (ADR 0029). */
function Axes({ axes }: { readonly axes: RunAxisValues }): ReactNode {
  const { i18n } = useLingui();
  // In an element of its own, so React replaces the element, never a text
  // node a page translator moved, when the axis gets its value (react#11538).
  const noneYet = (
    <span>
      <Trans>none yet</Trans>
    </span>
  );
  return (
    <dl className={listClassName}>
      <Fact term={<Trans>Lifecycle</Trans>}>{i18n._(LIFECYCLE_NAMES[axes.lifecycle])}</Fact>
      <Fact term={<Trans>Verdict</Trans>}>
        {axes.verdict === undefined || axes.verdict === null
          ? noneYet
          : i18n._(VERDICT_NAMES[axes.verdict])}
      </Fact>
      <Fact term={<Trans>Completeness</Trans>}>
        {axes.completeness === undefined || axes.completeness === null
          ? noneYet
          : i18n._(COMPLETENESS_NAMES[axes.completeness])}
      </Fact>
    </dl>
  );
}

/**
 * The worst storage level among the filesystems the run writes to (ADR
 * 0122). Each form is an element of its own, keyed, so React replaces the
 * element, never text nodes a page translator moved (react#11538).
 */
function Storage({ progress }: { readonly progress: RunProgress }): ReactNode {
  const { i18n } = useLingui();
  const { storage } = progress;
  if (storage === undefined || storage === null) {
    return <span key="normal">{i18n._(STORAGE_LEVEL_NAMES.normal)}</span>;
  }
  const level = i18n._(STORAGE_LEVEL_NAMES[storage.level]);
  const folder = storage.folder;
  const free = formatBytes(storage.free_bytes);
  return (
    <span key="level">
      <Trans>
        {level}: the filesystem of{" "}
        <code translate="no" className="font-mono">
          {folder}
        </code>{" "}
        has{" "}
        <data value={String(storage.free_bytes)} translate="no">
          {free}
        </data>{" "}
        free
      </Trans>
    </span>
  );
}

/** Who raised `pause`: the program, or the engine at one of its steps. */
function RaisedBy({ pause }: { readonly pause: HumanPause }): ReactNode {
  const origin = pause.raised_by;
  switch (origin.by) {
    case "program":
      return <Trans>Raised by the program.</Trans>;
    case "engine":
      return origin.step === "login_recovery" ? (
        <Trans>Raised by the engine at a login recovery.</Trans>
      ) : (
        <Trans>Raised by the engine at the Challenge ladder.</Trans>
      );
    default:
      return origin satisfies never;
  }
}

/** Each Human pause waiting (ADR 0092). */
function Pauses({ pauses }: { readonly pauses: readonly HumanPause[] }): ReactNode {
  const { t } = useLingui();
  if (pauses.length === 0) {
    return (
      <p>
        <Trans>No Human pause waits.</Trans>
      </p>
    );
  }
  return (
    <ul aria-label={t`Human pauses waiting`} className="flex flex-col gap-2">
      {pauses.map((pause) => {
        const { session, page } = pause;
        return (
          <li key={`${pause.session} ${pause.page}`} className="rounded-md border p-3">
            <p>
              <Trans>
                Page <code translate="no">{page}</code> of the Session{" "}
                <code translate="no">{session}</code> waits for a person since{" "}
                <Instant value={pause.since} />.
              </Trans>
            </p>
            {pause.reason === undefined || pause.reason === null ? null : (
              <p translate="no">{pause.reason}</p>
            )}
            <p className="text-sm text-muted-foreground">
              <RaisedBy pause={pause} />{" "}
              {pause.deadline === undefined || pause.deadline === null ? (
                <Trans>It waits without limit.</Trans>
              ) : (
                <Trans>
                  It ends unanswered at <Instant value={pause.deadline} />.
                </Trans>
              )}
            </p>
          </li>
        );
      })}
    </ul>
  );
}

/** Where the run is: its writer, run epoch, last Checkpoint, status line and storage level. */
function Progress({ progress }: { readonly progress: RunProgress }): ReactNode {
  const epoch = formatCount(progress.epoch);
  return (
    <dl className={listClassName}>
      <Fact term={<Trans>Writer</Trans>}>
        {progress.writer === "live" ? (
          <Trans>live: an engine works this run</Trans>
        ) : (
          <Trans>none: no engine works this run</Trans>
        )}
      </Fact>
      <Fact term={<Trans>Run epoch</Trans>}>
        <data value={String(progress.epoch)} translate="no">
          {epoch}
        </data>
      </Fact>
      <Fact term={<Trans>Last Checkpoint</Trans>}>
        {progress.checkpoint_at === undefined || progress.checkpoint_at === null ? (
          <span>
            <Trans>none yet</Trans>
          </span>
        ) : (
          <Instant value={progress.checkpoint_at} />
        )}
      </Fact>
      <Fact term={<Trans>Status</Trans>}>
        {progress.message === undefined || progress.message === null ? (
          <span>
            <Trans>no status line</Trans>
          </span>
        ) : (
          <span translate="no">{progress.message}</span>
        )}
      </Fact>
      <Fact term={<Trans>Storage</Trans>}>
        <Storage progress={progress} />
      </Fact>
    </dl>
  );
}

/** What the last action came to, announced. */
function NoticeText({ notice }: { readonly notice: Notice }): ReactNode {
  switch (notice.kind) {
    case "checkpoint_written":
      return (
        <Trans>
          Checkpoint written at <Instant value={notice.at} />.
        </Trans>
      );
    case "stopped":
      return <Trans>The run stopped.</Trans>;
    case "copied":
      return <Trans>Copied the resume command.</Trans>;
    case "copy_refused":
      return <Trans>The browser refused to copy the resume command: select it and copy it.</Trans>;
    case "refused":
      return (
        <>
          <p>
            <Trans>The engine refused:</Trans>
          </p>
          <ErrorMessage error={notice.error} />
        </>
      );
    default:
      return notice satisfies never;
  }
}

/** The actions a person can take on the run: stop and checkpoint while it is live, its resume command once it is interrupted or stopped. */
function Actions({
  runId,
  run,
  readOnly,
  busy,
  notice,
  onStop,
  onCheckpoint,
  onCopy,
}: Omit<RunPageViewProps, "read"> & { readonly run: ShownRun }): ReactNode {
  const command = resumeCommand(runId, run.needs);
  return (
    <>
      {live(run) ? (
        <div className="flex gap-3">
          <button
            type="button"
            disabled={readOnly === true || busy === true}
            onClick={onCheckpoint}
            className={buttonClassName}
          >
            <Trans>Write a Checkpoint</Trans>
          </button>
          <button
            type="button"
            disabled={readOnly === true || busy === true}
            onClick={onStop}
            className={buttonClassName}
          >
            <Trans>Stop</Trans>
          </button>
        </div>
      ) : null}
      {resumable(run) ? (
        <div className="flex flex-col gap-2">
          <p>
            {interrupted(run) ? (
              <Trans>
                No engine works this run: it was interrupted. Run this command to resume it from its
                committed state:
              </Trans>
            ) : (
              <Trans>This run stopped. Run this command to resume it:</Trans>
            )}
          </p>
          <code translate="no" className="self-start rounded-md bg-muted px-2 py-1 font-mono">
            {command}
          </code>
          <button
            type="button"
            onClick={() => onCopy?.(command)}
            className={`${buttonClassName} self-start`}
          >
            <Trans>Copy resume command</Trans>
          </button>
        </div>
      ) : null}
      {/* Each notice is an element of its own, keyed by its kind, so React replaces the element, never text nodes a page translator moved (react#11538). */}
      <div role="status">
        {notice === undefined ? null : (
          <div key={notice.kind}>
            <NoticeText notice={notice} />
          </div>
        )}
      </div>
    </>
  );
}

/** The run, as the page read it. */
function Shown({
  run,
  ...props
}: Omit<RunPageViewProps, "read"> & { readonly run: ShownRun }): ReactNode {
  return (
    <>
      {run.uncapped ? (
        <p className="self-start rounded-md border border-destructive px-3 py-1.5">
          <strong className="text-destructive">
            <Trans>Uncapped</Trans>
          </strong>{" "}
          <Trans>This run can spend money with no Limit capping it.</Trans>
        </p>
      ) : null}
      <section aria-labelledby="run-axes" className={sectionClassName}>
        <h2 id="run-axes" className="text-xl font-semibold">
          <Trans>How it stands</Trans>
        </h2>
        <Axes axes={run.axes} />
      </section>
      <section aria-labelledby="run-progress" className={sectionClassName}>
        <h2 id="run-progress" className="text-xl font-semibold">
          <Trans>Progress</Trans>
        </h2>
        <Progress progress={run.progress} />
        <h3 className="font-semibold">
          <Trans>Human pauses</Trans>
        </h3>
        <Pauses pauses={run.progress.pauses} />
        <Actions run={run} {...props} />
      </section>
    </>
  );
}

/** What the page read, shown. */
function Read({ read, ...props }: RunPageViewProps): ReactNode {
  const { runId } = props;
  switch (read.kind) {
    case "reading":
      return (
        <p role="status">
          <Trans>Reading the run…</Trans>
        </p>
      );
    case "not_listed":
      return (
        <p role="alert">
          <Trans>
            No run has the Run ID <code translate="no">{runId}</code>: the Run index does not list
            it.
          </Trans>
        </p>
      );
    case "failed":
      return (
        <div role="alert">
          <p>
            <Trans>The engine could not read the run:</Trans>
          </p>
          <ErrorMessage error={read.error} />
        </div>
      );
    case "shown":
      return <Shown run={read.run} {...props} />;
    default:
      return read satisfies never;
  }
}

/** A run's page, as it shows `read`. */
export function RunPageView(props: RunPageViewProps): ReactNode {
  const { runId } = props;
  return (
    <div className="flex flex-col gap-6">
      <a
        href={RUNS}
        className="self-start rounded-sm underline-offset-4 hover:underline focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none"
      >
        <Trans>All runs</Trans>
      </a>
      <h1 className="font-heading text-3xl font-semibold tracking-tight">
        <Trans>
          Run <code translate="no">{runId}</code>
        </Trans>
      </h1>
      <Read {...props} />
    </div>
  );
}

/** The field `name` of `value`, if it is an object that has one. */
function member(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

/** What the page holds: what it read, and how many times something woke it to read it all again. */
interface PageState {
  readonly read: RunRead;
  readonly wakes: number;
}

type PageAction =
  | { readonly kind: "read"; readonly read: RunRead }
  /** The run's axes changed, or the stream resynced. */
  | { readonly kind: "woken" };

function reduce(state: PageState, action: PageAction): PageState {
  switch (action.kind) {
    case "read":
      return { ...state, read: action.read };
    case "woken":
      // While it follows the run, the page reads every change by itself.
      return state.read.kind === "shown" && !state.read.run.progress.last
        ? state
        : { ...state, wakes: state.wakes + 1 };
    default:
      return action satisfies never;
  }
}

/** What a refused read of the run shows. */
function refusedRead(error: unknown): RunRead {
  if (error instanceof GoodfetchError && error.code === "run_not_found") {
    return { kind: "not_listed" };
  }
  return { kind: "failed", error };
}

/** The run's Run index row; none when the index has no row of it or the read fails. */
async function rowOf(
  client: ConsoleClient,
  runId: string,
  signal: AbortSignal,
): Promise<RunIndexRow | undefined> {
  try {
    const { runs } = await client.call("run.list", { text: runId, watch_runs: true }, { signal });
    return runs.find((row) => row.run_id === runId);
  } catch {
    signal.throwIfAborted();
    return undefined;
  }
}

/** Whether the run is uncapped; none when its Usage cannot be read. */
async function uncappedOf(
  client: ConsoleClient,
  runId: string,
  signal: AbortSignal,
): Promise<boolean | undefined> {
  try {
    const usage = await client.call("run.usage", { run_id: runId }, { signal });
    return usage.uncapped === true;
  } catch {
    signal.throwIfAborted();
    return undefined;
  }
}

/** The run `progress` and the reads beside it show, after `before`. */
function shownOf(
  progress: RunProgress,
  row: RunIndexRow | undefined,
  uncapped: boolean | undefined,
  before: ShownRun | undefined,
): ShownRun {
  const axes: RunAxisValues =
    row === undefined
      ? before?.axes.lifecycle === progress.lifecycle
        ? before.axes
        : { lifecycle: progress.lifecycle }
      : { lifecycle: progress.lifecycle, verdict: row.verdict, completeness: row.completeness };
  return {
    progress,
    axes,
    needs: row?.needs ?? before?.needs ?? "collie",
    uncapped: uncapped ?? before?.uncapped ?? false,
  };
}

/** A run's page over `client`. */
export function RunPage({
  client,
  runId,
  readOnly,
}: {
  readonly client: ConsoleClient;
  readonly runId: string;
  readonly readOnly: boolean;
}): ReactNode {
  const connection = useSyncExternalStore(
    (changed) => client.watch(changed),
    () => client.state,
  );
  const [{ read, wakes }, dispatch] = useReducer(reduce, {
    read: { kind: "reading" },
    wakes: 0,
  });
  const [{ busy, notice }, setAction] = useState<{
    readonly busy: boolean;
    readonly notice: Notice | undefined;
  }>({ busy: false, notice: undefined });

  useEffect(() => {
    if (connection !== "connected") {
      return undefined;
    }
    const abort = new AbortController();
    const { signal } = abort;
    const follow = async (): Promise<void> => {
      const [progress, row, uncapped] = await Promise.all([
        client.call("run.progress", { run_id: runId }, { signal }),
        rowOf(client, runId, signal),
        uncappedOf(client, runId, signal),
      ]);
      let shown = shownOf(progress, row, uncapped, undefined);
      dispatch({ kind: "read", read: { kind: "shown", run: shown } });
      while (!shown.progress.last) {
        const next = await client.call(
          "run.progress",
          { run_id: runId, after: shown.progress.position },
          { signal },
        );
        const [nextRow, nextUncapped] = await Promise.all([
          next.lifecycle === shown.axes.lifecycle ? undefined : rowOf(client, runId, signal),
          uncappedOf(client, runId, signal),
        ]);
        shown = shownOf(next, nextRow, nextUncapped, shown);
        dispatch({ kind: "read", read: { kind: "shown", run: shown } });
      }
    };
    follow().catch((error: unknown) => {
      if (!signal.aborted) {
        dispatch({ kind: "read", read: refusedRead(error) });
      }
    });
    return () => abort.abort();
  }, [client, runId, connection, wakes]);

  useEffect(() => {
    const subscription = client.subscribe("events");
    const listen = async (): Promise<void> => {
      for await (const item of subscription) {
        if (
          item.kind === "resync" ||
          (item.event === "run.axes_changed" && member(item.data, "run_id") === runId)
        ) {
          dispatch({ kind: "woken" });
        }
      }
    };
    void listen();
    return () => subscription.close();
  }, [client, runId]);

  const act = async (action: () => Promise<Notice>): Promise<void> => {
    setAction({ busy: true, notice: undefined });
    let done: Notice;
    try {
      done = await action();
    } catch (error) {
      done = { kind: "refused", error };
    }
    setAction({ busy: false, notice: done });
  };

  return (
    <RunPageView
      runId={runId}
      read={read}
      readOnly={readOnly}
      busy={busy}
      notice={notice}
      onStop={() =>
        void act(async () => {
          await client.call("run.stop", { run_id: runId });
          return { kind: "stopped" };
        })
      }
      onCheckpoint={() =>
        void act(async () => {
          const checkpoint = await client.call("run.checkpoint", { run_id: runId });
          return { kind: "checkpoint_written", at: checkpoint.time };
        })
      }
      onCopy={(command) =>
        void act(async () => {
          try {
            await navigator.clipboard.writeText(command);
          } catch {
            return { kind: "copy_refused" };
          }
          return { kind: "copied" };
        })
      }
    />
  );
}
