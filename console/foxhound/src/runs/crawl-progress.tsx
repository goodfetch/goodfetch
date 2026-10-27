// A crawl run's Progress view (spec #593, story 217; ADRs 0102 and 0118;
// ADR 0037 / RT-15): it follows the run through `run.progress`, each read
// after the position it last read, until the last value, and shows the
// crawl's parts of each value, so a slow crawl explains itself: the
// program's status message and whether a keep-alive crawl is idle; each
// request in flight with its phase, tier and attempt; why due requests
// wait, one group per reason, with the domains held, when each is next
// admitted and the setting behind the wait with its value and layer; each
// sitemap seed's progress; and the handlers running against their
// concurrency, with the Resource governor's level. A part the engine has
// not sent shows as none.
import type { RunViewProps } from "@goodfetch/console/console-build";
import { ErrorMessage } from "@goodfetch/console/error-message";
import { formatCount, formatInstant } from "@goodfetch/console/format";
import { isSettingsLayer, LAYERS } from "@goodfetch/console/settings-layers";
import type { GovernorLevel } from "@goodfetch/collie-client/generated";
import type { SitemapReport } from "@goodfetch/foxhound/generated";
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import { Plural, Trans, useLingui } from "@lingui/react/macro";
import { useEffect, useId, useState, type ReactNode } from "react";
import {
  crawlProgressOf,
  type CrawlConcurrency,
  type CrawlRunProgress,
  type InFlightPhase,
  type InFlightRequest,
  type WaitGroup,
} from "./crawl-parts.ts";

/** What the view read of the run's progress. */
type Followed =
  | { readonly kind: "reading" }
  | { readonly kind: "refused"; readonly error: unknown }
  | { readonly kind: "shown"; readonly progress: CrawlRunProgress };

/** Follows the run `runId`'s progress, from its value now to its last. */
function useCrawlProgress(client: RunViewProps["client"], runId: string): Followed {
  const [followed, setFollowed] = useState<Followed>({ kind: "reading" });
  useEffect(() => {
    const abort = new AbortController();
    const { signal } = abort;
    const follow = async (): Promise<void> => {
      try {
        let progress = await client.call("run.progress", { run_id: runId }, { signal });
        for (;;) {
          signal.throwIfAborted();
          setFollowed({ kind: "shown", progress: crawlProgressOf(progress) });
          if (progress.last) {
            return;
          }
          progress = await client.call(
            "run.progress",
            { run_id: runId, after: progress.position },
            { signal },
          );
        }
      } catch (error) {
        if (!signal.aborted) {
          setFollowed({ kind: "refused", error });
        }
      }
    };
    void follow();
    return () => abort.abort();
  }, [client, runId]);
  return followed;
}

/** The name of each phase of a request in flight. */
const PHASE_NAMES: Readonly<Record<InFlightPhase, MessageDescriptor>> = {
  fetching: msg`Fetching`,
  handling: msg`Handling`,
  waiting_for_permit: msg`Waiting for a navigation permit`,
};

/** The name of each reason the engine of this version gives a wait. */
const REASON_NAMES: Readonly<Record<string, MessageDescriptor>> = {
  domain_concurrency: msg`Domain concurrency`,
  domain_delay: msg`Domain delay`,
  crawl_delay: msg`Crawl-delay of robots.txt`,
  backoff: msg`Back-off`,
  robots_txt: msg`robots.txt not read yet`,
  handler_concurrency: msg`Handler concurrency`,
  governor: msg`Resource governor`,
  session: msg`Waiting for a Session`,
  human_pause: msg`Human pause`,
};

/** The name of each level of the Resource governor. */
const GOVERNOR_NAMES: Readonly<Record<GovernorLevel, MessageDescriptor>> = {
  normal: msg`Normal`,
  pressure: msg`Pressure`,
  critical: msg`Critical`,
};

const sectionClassName = "flex flex-col gap-2";
const headingClassName = "text-lg font-semibold";
const tableClassName = "w-full text-start";
const cellClassName = "border-b px-2 py-1 text-start align-top";

function Instant({ value }: { readonly value: string }): ReactNode {
  return (
    <time dateTime={value} translate="no">
      {formatInstant(value)}
    </time>
  );
}

/** A reason's name: the catalog's words for a known one, the engine's name for a newer one. */
function useReasonName(): (reason: string) => ReactNode {
  const { i18n } = useLingui();
  return (reason) => {
    const name = REASON_NAMES[reason];
    return name === undefined ? reason : i18n._(name);
  };
}

/** The program's status message and whether a keep-alive crawl is idle. */
function Status({ progress }: { readonly progress: CrawlRunProgress }): ReactNode {
  const id = useId();
  return (
    <section aria-labelledby={id} className={sectionClassName}>
      <h2 id={id} className={headingClassName}>
        <Trans>Status</Trans>
      </h2>
      {progress.message === undefined || progress.message === null ? (
        <p>
          <Trans>The program set no status message.</Trans>
        </p>
      ) : (
        <p translate="no">{progress.message}</p>
      )}
      {progress.idle === true ? (
        <p>
          <Trans>Idle: the crawl waits for new work, held running by crawl.keep_alive.</Trans>
        </p>
      ) : null}
      {progress.last ? (
        <p>
          <Trans>The crawl ended: nothing is in flight or waits any more.</Trans>
        </p>
      ) : null}
    </section>
  );
}

/** Each request in flight, with its phase, tier and attempt. */
function InFlight({ requests }: { readonly requests: readonly InFlightRequest[] }): ReactNode {
  const { i18n } = useLingui();
  if (requests.length === 0) {
    return (
      <p>
        <Trans>No request is in flight.</Trans>
      </p>
    );
  }
  return (
    <table className={tableClassName}>
      <caption className="text-start font-semibold">
        <Trans>Requests in flight</Trans>
      </caption>
      <thead>
        <tr>
          <th scope="col" className={cellClassName}>
            <Trans>URL</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Phase</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Tier</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Attempt</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Attempt began</Trans>
          </th>
        </tr>
      </thead>
      <tbody>
        {requests.map((request) => (
          <tr key={request.request_id}>
            <td className={`${cellClassName} break-all`} translate="no">
              {request.url}
            </td>
            <td className={cellClassName}>{i18n._(PHASE_NAMES[request.phase])}</td>
            <td className={cellClassName} translate="no">
              {request.tier ?? ""}
            </td>
            <td className={cellClassName}>{formatCount(request.attempt)}</td>
            <td className={cellClassName}>
              <Instant value={request.started_at} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** The setting behind a wait, with its value and the layer that set it. */
function SettingBehind({ group }: { readonly group: WaitGroup }): ReactNode {
  const { i18n } = useLingui();
  const { setting } = group;
  if (setting === undefined || setting === null) {
    return (
      <p>
        <Trans>No setting sets this wait.</Trans>
      </p>
    );
  }
  const key = setting.key;
  const value = JSON.stringify(setting.value);
  const layer = isSettingsLayer(setting.layer) ? i18n._(LAYERS[setting.layer]) : setting.layer;
  return (
    <p>
      <Trans>
        Set by <span translate="no">{key}</span>: <span translate="no">{value}</span>, from {layer}
      </Trans>
    </p>
  );
}

/** The due requests one reason holds, the domains it holds and the setting behind it. */
function Wait({ group }: { readonly group: WaitGroup }): ReactNode {
  const id = useId();
  const reasonName = useReasonName();
  const reason = reasonName(group.reason);
  const more = group.more_domains ?? 0;
  return (
    <section aria-labelledby={id} className={sectionClassName}>
      <h3 id={id} className="font-semibold">
        {reason}: <Plural value={group.count} one="# request waits" other="# requests wait" />
      </h3>
      {group.domains.length === 0 ? null : (
        <table className={tableClassName}>
          <caption className="sr-only">
            <Trans>Domains {reason} holds</Trans>
          </caption>
          <thead>
            <tr>
              <th scope="col" className={cellClassName}>
                <Trans>Domain</Trans>
              </th>
              <th scope="col" className={cellClassName}>
                <Trans>Next admitted</Trans>
              </th>
            </tr>
          </thead>
          <tbody>
            {group.domains.map((held) => (
              <tr key={held.domain}>
                <td className={cellClassName} translate="no">
                  {held.domain}
                </td>
                <td className={cellClassName}>
                  {held.next_admitted_at === undefined || held.next_admitted_at === null ? (
                    <Trans>Once a request in flight ends</Trans>
                  ) : (
                    <Instant value={held.next_admitted_at} />
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
      {more === 0 ? null : (
        <p>
          <Plural value={more} one="and # more domain" other="and # more domains" />
        </p>
      )}
      <SettingBehind group={group} />
    </section>
  );
}

/** Why due requests wait, one group per reason. */
function Waiting({ groups }: { readonly groups: readonly WaitGroup[] }): ReactNode {
  const id = useId();
  return (
    <section aria-labelledby={id} className={sectionClassName}>
      <h2 id={id} className={headingClassName}>
        <Trans>Why due requests wait</Trans>
      </h2>
      {groups.length === 0 ? (
        <p>
          <Trans>No due request waits.</Trans>
        </p>
      ) : (
        groups.map((group) => <Wait key={group.reason} group={group} />)
      )}
    </section>
  );
}

/** Each sitemap seed's sitemaps read and failed, and its entries added and not. */
function Sitemaps({ sitemaps }: { readonly sitemaps: readonly SitemapReport[] }): ReactNode {
  if (sitemaps.length === 0) {
    return null;
  }
  return (
    <table className={tableClassName}>
      <caption className="text-start font-semibold">
        <Trans>Sitemap seeds</Trans>
      </caption>
      <thead>
        <tr>
          <th scope="col" className={cellClassName}>
            <Trans>Seed</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Sitemaps read</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Sitemaps failed</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Entries added</Trans>
          </th>
          <th scope="col" className={cellClassName}>
            <Trans>Entries not added</Trans>
          </th>
        </tr>
      </thead>
      <tbody>
        {sitemaps.map((sitemap) => (
          <tr key={sitemap.seed}>
            <td className={`${cellClassName} break-all`} translate="no">
              {sitemap.seed}
            </td>
            <td className={cellClassName}>{formatCount(sitemap.read)}</td>
            <td className={cellClassName}>{formatCount(sitemap.failed)}</td>
            <td className={cellClassName}>{formatCount(sitemap.added)}</td>
            <td className={cellClassName}>{formatCount(sitemap.not_added)}</td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}

/** The handlers running against their concurrency, and the Resource governor's level. */
function Concurrency({
  concurrency,
}: {
  readonly concurrency: CrawlConcurrency | null | undefined;
}): ReactNode {
  const id = useId();
  const { i18n } = useLingui();
  if (concurrency === undefined || concurrency === null) {
    return null;
  }
  const handlers = formatCount(concurrency.handlers);
  const allowed = formatCount(concurrency.handler_concurrency);
  const { governor, open_pages: open, page_allowance: allowance } = concurrency;
  return (
    <section aria-labelledby={id} className={sectionClassName}>
      <h2 id={id} className={headingClassName}>
        <Trans>Concurrency</Trans>
      </h2>
      <dl className="grid grid-cols-[max-content_1fr] gap-x-4 gap-y-1">
        <dt>
          <Trans>Handlers running</Trans>
        </dt>
        <dd>
          <Trans>
            {handlers} of {allowed}
          </Trans>
        </dd>
        <dt>
          <Trans>Resource governor</Trans>
        </dt>
        <dd>
          {governor === undefined || governor === null ? (
            <Trans>Not sampled yet</Trans>
          ) : (
            i18n._(GOVERNOR_NAMES[governor])
          )}
        </dd>
        {open === undefined || open === null ? null : (
          <>
            <dt>
              <Trans>Browser pages open</Trans>
            </dt>
            <dd>
              {allowance === undefined || allowance === null ? (
                formatCount(open)
              ) : (
                <Trans>
                  {formatCount(open)} of {formatCount(allowance)} allowed
                </Trans>
              )}
            </dd>
          </>
        )}
      </dl>
    </section>
  );
}

/** A crawl run's progress, followed live. */
export function CrawlProgress({ client, runId }: RunViewProps): ReactNode {
  const followed = useCrawlProgress(client, runId);
  if (followed.kind === "refused") {
    return <ErrorMessage error={followed.error} />;
  }
  if (followed.kind === "reading") {
    return (
      <output className="block">
        <Trans>Asking the engine…</Trans>
      </output>
    );
  }
  const { progress } = followed;
  return (
    <div className="flex flex-col gap-4">
      <Status progress={progress} />
      <InFlight requests={progress.in_flight ?? []} />
      <Waiting groups={progress.waiting ?? []} />
      <Sitemaps sitemaps={progress.sitemaps ?? []} />
      <Concurrency concurrency={progress.concurrency} />
    </div>
  );
}
