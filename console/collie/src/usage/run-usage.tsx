// A run's Usage, the part of Usage and Limits a run's page shows (spec
// #591, story 99; ADRs 0041, 0046 and 0105): what the run used of each
// Usage kind, from its Run report's `usage` (or `run.usage`), and the
// Limits that apply to it with what is left. The responses a Response
// recording answered in place of a fetch are counted apart from the
// requests the run fetched, from the report's `recording`, so a replayed
// test run never reads as traffic to a site, nor counts against a Limit on
// requests.
import type { RecordingReport, RunUsage, UsageAmount } from "@goodfetch/collie-client/generated";
import { Trans } from "@lingui/react/macro";
import type { ReactNode } from "react";
import { formatAmount } from "#format.ts";
import { LimitsTable, type LimitRow } from "./limits.tsx";

/** What a run's page gives: the run's Usage, and what it did with its Response recording, if its start named one. */
export interface RunUsageViewProps {
  readonly usage: RunUsage;
  readonly recording: RecordingReport | null | undefined;
}

/** One line of what the run used. */
interface Used {
  readonly key: string;
  readonly name: ReactNode;
  readonly amount: number;
  readonly currency?: string | null | undefined;
}

/** The responses the run's recording answered: none unless it replayed. */
function replayedOf(recording: RecordingReport | null | undefined): number {
  return recording?.mode === "replay" ? recording.responses_used : 0;
}

/** A line per currency of a priced Usage kind. */
function priced(
  kind: string,
  amounts: readonly UsageAmount[] | undefined,
  name: (currency: string) => ReactNode,
): Used[] {
  return (amounts ?? []).map(({ currency, amount }) => ({
    key: `${kind}-${currency ?? ""}`,
    name:
      currency === undefined || currency === null ? (
        <code translate="no">{kind}</code>
      ) : (
        name(currency)
      ),
    amount,
    currency,
  }));
}

/** Every line of `usage`, the replayed responses beside the requests fetched. */
function lines(usage: RunUsage, recording: RecordingReport | null | undefined): Used[] {
  return [
    { key: "requests", name: <Trans>Requests fetched</Trans>, amount: usage.requests },
    { key: "replayed", name: <Trans>Responses replayed</Trans>, amount: replayedOf(recording) },
    {
      key: "browser_time",
      name: <Trans>Browser time, in milliseconds</Trans>,
      amount: usage.browser_time,
    },
    { key: "proxy_bytes", name: <Trans>Proxy bytes</Trans>, amount: usage.proxy_bytes },
    { key: "model_calls", name: <Trans>Model calls</Trans>, amount: usage.model.calls },
    { key: "model_tokens", name: <Trans>Model tokens</Trans>, amount: usage.model.tokens },
    ...priced("paid_solves", usage.paid_solves, (currency) => (
      <Trans>
        Paid solves in <span translate="no">{currency}</span>
      </Trans>
    )),
    ...priced("paid_fetch", usage.paid_fetches, (currency) => (
      <Trans>
        Paid fetches in <span translate="no">{currency}</span>
      </Trans>
    )),
    ...(usage.other ?? []).map(({ usage_kind, currency, amount }) => ({
      key: `${usage_kind}-${currency ?? ""}`,
      name:
        currency === undefined || currency === null ? (
          <code translate="no">{usage_kind}</code>
        ) : (
          <Trans>
            <code translate="no">{usage_kind}</code> in <span translate="no">{currency}</span>
          </Trans>
        ),
      amount,
      currency,
    })),
  ];
}

/** What a run used, and the Limits that apply to it. */
export function RunUsageView({ usage, recording }: RunUsageViewProps): ReactNode {
  const limits: LimitRow[] = (usage.limits ?? []).map((limit) => ({
    scope: limit.scope,
    usageKind: limit.usage_kind,
    currency: limit.currency,
    period: limit.period,
    resetsAt: limit.resets_at,
    amount: limit.amount,
    left: limit.left,
    setBy: limit.set_by,
  }));
  return (
    <div className="flex flex-col gap-4">
      <table className="w-full border-collapse text-start text-sm">
        <caption className="sr-only">
          <Trans>What this run used</Trans>
        </caption>
        <tbody>
          {lines(usage, recording).map((line) => (
            <tr key={line.key} className="border-b">
              <th scope="row" className="py-2 pe-4 font-normal">
                {line.name}
              </th>
              <td className="py-2 tabular-nums">
                <data value={String(line.amount)} translate="no">
                  {formatAmount(line.amount, line.currency)}
                </data>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="text-muted-foreground text-sm">
        <Trans>
          Responses replayed from the run's Response recording reached no site and count against no
          Limit.
        </Trans>
      </p>
      {limits.length === 0 ? (
        <p>
          <Trans>No Limit applies to this run.</Trans>
        </p>
      ) : (
        <LimitsTable rows={limits} caption={<Trans>Limits that apply to this run</Trans>} />
      )}
    </div>
  );
}
