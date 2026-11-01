// Foxhound's views of a run that needs Foxhound (spec #593, stories 203
// and 217; ADRs 0087 and 0098), which Foxhound's build entry passes the
// shell, so a run's page shows each in a tab beside the views every run
// has: the crawl's progress, followed live (`crawl-progress.tsx`), the
// crawl's seeds, as `run.seeds` answers them, the request log of each
// crawl request's outcome, followed live (`request-log.tsx`), the requests
// whose latest outcome failed, as `run.failed_requests` lists them, and the
// SQL tab over the runs a person selects (`#sql-tab/sql-tab.tsx`). Each view
// reads its operation once it shows, and shows the engine's refusal as the
// Console shows any. A Collie run shows none of them: its page has only
// the views every run has.
import type { RunView, RunViewProps } from "@goodfetch/console/console-build";
import { ErrorMessage } from "@goodfetch/console/error-message";
import type { CrawlRequestOutcome, Enqueued } from "@goodfetch/foxhound/generated";
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import { Plural, Trans, useLingui } from "@lingui/react/macro";
import { useEffect, useState, type ReactNode } from "react";
import { callFoxhound } from "#call.ts";
import { Datasets } from "#datasets/datasets.tsx";
import { CrawlProgress } from "#runs/crawl-progress.tsx";
import { RequestLog } from "#runs/request-log.tsx";
import type { FoxhoundConsoleOperations } from "#generated/operations.ts";
import { SQL_TAB_VIEW } from "#sql-tab/sql-tab.tsx";

/** What a view read of its operation. */
type Read<Result> =
  | { readonly kind: "reading" }
  | { readonly kind: "refused"; readonly error: unknown }
  | { readonly kind: "answered"; readonly result: Result };

/** The operations a view reads, each taking only the run's Run ID. */
type RunRead = "run.seeds" | "run.failed_requests";

/** What `operation` answers of the run `runId`, read once the view shows. */
function useRunRead<Name extends RunRead>(
  client: RunViewProps["client"],
  runId: string,
  operation: Name,
): Read<FoxhoundConsoleOperations[Name]["result"]> {
  const [read, setRead] = useState<Read<FoxhoundConsoleOperations[Name]["result"]>>({
    kind: "reading",
  });
  useEffect(() => {
    const abort = new AbortController();
    const { signal } = abort;
    const ask = async (): Promise<void> => {
      try {
        const result = await callFoxhound(client, operation, { run_id: runId }, signal);
        if (!signal.aborted) {
          setRead({ kind: "answered", result });
        }
      } catch (error) {
        if (!signal.aborted) {
          setRead({ kind: "refused", error });
        }
      }
    };
    void ask();
    return () => abort.abort();
  }, [client, runId, operation]);
  return read;
}

/** What a view shows while it reads, or once the engine refused. */
function Pending({ read }: { readonly read: Read<unknown> }): ReactNode {
  return read.kind === "refused" ? (
    <ErrorMessage error={read.error} />
  ) : (
    <output className="block">
      <Trans>Asking the engine…</Trans>
    </output>
  );
}

/** The name of each kind of outcome. */
const OUTCOME_NAMES: Readonly<Record<CrawlRequestOutcome["kind"], MessageDescriptor>> = {
  succeeded: msg`Succeeded`,
  failed: msg`Failed`,
  skipped: msg`Skipped`,
  blocked: msg`Blocked`,
};

const tableClassName = "w-full text-start";
const cellClassName = "border-b px-2 py-1 text-start align-top";

/** A table of crawl requests' outcomes, each with its URL, Label and outcome. */
function OutcomeTable({
  caption,
  outcomes,
}: {
  readonly caption: ReactNode;
  readonly outcomes: readonly CrawlRequestOutcome[];
}): ReactNode {
  const { i18n } = useLingui();
  return (
    <table className={tableClassName}>
      <caption className="text-start font-semibold">{caption}</caption>
      <thead>
        <tr>
          <th scope="col" className={cellClassName}>
            <Trans>URL</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Label</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Outcome</Trans>
          </th>
        </tr>
      </thead>
      <tbody>
        {outcomes.map((outcome) => (
          <tr key={outcome.position}>
            <td className={`${cellClassName} break-all`} translate="no">
              {outcome.request.url}
            </td>
            <td className={cellClassName} translate="no">
              {outcome.request.label}
            </td>
            <td className={cellClassName}>{i18n._(OUTCOME_NAMES[outcome.kind])}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** The crawl's seeds, each added or not. */
function Seeds({ client, runId }: RunViewProps): ReactNode {
  const read = useRunRead(client, runId, "run.seeds");
  if (read.kind !== "answered") {
    return <Pending read={read} />;
  }
  const { seeds } = read.result;
  return (
    <table className={tableClassName}>
      <caption className="text-start font-semibold">
        <Plural value={seeds.length} one="# seed" other="# seeds" />
      </caption>
      <thead>
        <tr>
          <th scope="col" className={cellClassName}>
            <Trans>URL</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Added</Trans>
          </th>
        </tr>
      </thead>
      <tbody>
        {seeds.map((seed: Enqueued, index) => (
          // A seed's place in the start is its identity: two seeds may name one URL,
          // and the seeds keep the order the start gave them.
          <tr key={index}>
            <td className={`${cellClassName} break-all`} translate="no">
              {seed.url}
            </td>
            <td className={cellClassName}>
              {seed.kind === "added" ? <Trans>Added</Trans> : <Trans>Not added</Trans>}
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** The requests whose latest outcome failed. */
function FailedRequests({ client, runId }: RunViewProps): ReactNode {
  const read = useRunRead(client, runId, "run.failed_requests");
  if (read.kind !== "answered") {
    return <Pending read={read} />;
  }
  const { requests } = read.result;
  return (
    <OutcomeTable
      caption={<Plural value={requests.length} one="# failed request" other="# failed requests" />}
      outcomes={requests}
    />
  );
}

/** Foxhound's views of a run that needs Foxhound, in their tabs' order. */
export const FOXHOUND_RUN_VIEWS: readonly RunView[] = [
  { id: "progress", name: msg`Progress`, content: (props) => <CrawlProgress {...props} /> },
  { id: "seeds", name: msg`Seeds`, content: (props) => <Seeds {...props} /> },
  { id: "outcomes", name: msg`Outcomes`, content: (props) => <RequestLog {...props} /> },
  {
    id: "failed-requests",
    name: msg`Failed requests`,
    content: (props) => <FailedRequests {...props} />,
  },
  { id: "datasets", name: msg`Datasets`, content: (props) => <Datasets {...props} /> },
  SQL_TAB_VIEW,
];
