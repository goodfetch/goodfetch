// The Limits a person reads Usage against (ADR 0041), one row each, by
// scope: the install, a Daemon, a person, a named token, a Client
// connection, then any scope a newer engine wrote. Each row shows the Usage
// kind it caps (in its currency, for a price Limit), its period in its time
// zone and when it starts again, what it allows and what is left, who set
// it, and, read from the Usage ledger, the Usage charged and held against
// it and whether this engine understands a row flagged *must understand*.
// Engine & settings shows the ledger's rows; a run's page shows the rows
// that apply to its run.
import type {
  LimitScope,
  LimitSetter,
  LimitStanding,
  Period,
  PointInTime,
} from "@goodfetch/collie-client/generated";
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import { Trans, useLingui } from "@lingui/react/macro";
import type { ReactNode } from "react";
import { formatAmount, formatInstant } from "#format.ts";

/** What the ledger adds to a Limit: the Usage against it and what this engine makes of it. */
export interface LedgerUsage {
  readonly charged: number;
  readonly held: number;
  readonly mustUnderstand: boolean;
  readonly standing: LimitStanding;
}

/** One Limit as a row shows it. */
export interface LimitRow {
  readonly scope: LimitScope;
  readonly usageKind: string;
  readonly currency?: string | null | undefined;
  readonly period?: Period | null | undefined;
  readonly resetsAt?: PointInTime | null | undefined;
  readonly amount: number;
  readonly left: number;
  readonly setBy: LimitSetter;
  /** Read from the Usage ledger; none on a run's page. */
  readonly ledger?: LedgerUsage | undefined;
}

/** The order scopes are listed in: the widest first, then any other by name. */
const SCOPE_ORDER: readonly string[] = ["install", "daemon", "person", "token", "connection"];

function scopeRank(kind: string): number {
  const rank = SCOPE_ORDER.indexOf(kind);
  return rank === -1 ? SCOPE_ORDER.length : rank;
}

/** `rows`, by scope, then by scope ID and Usage kind. */
export function byScope(rows: readonly LimitRow[]): LimitRow[] {
  return rows.toSorted(
    (left, right) =>
      scopeRank(left.scope.kind) - scopeRank(right.scope.kind) ||
      left.scope.kind.localeCompare(right.scope.kind) ||
      left.scope.id.localeCompare(right.scope.id) ||
      left.usageKind.localeCompare(right.usageKind) ||
      (left.currency ?? "").localeCompare(right.currency ?? ""),
  );
}

/** Who set a Limit. */
const SETTERS: Readonly<Record<LimitSetter, MessageDescriptor>> = {
  person: msg`A person`,
  managed_policy: msg`Managed Operator policy`,
  client: msg`A Client`,
};

/** Whether a row must be understood, and what this engine makes of it. */
const UNDERSTANDING: Readonly<Record<`${boolean}-${LimitStanding}`, MessageDescriptor>> = {
  "true-held": msg`Yes, understood`,
  "true-not_understood": msg`Yes, not understood by this engine: runs it may cover are refused`,
  "true-foxhound": msg`Yes, Foxhound's, kept as written`,
  "false-held": msg`No`,
  "false-not_understood": msg`No, not understood by this engine: ignored`,
  "false-foxhound": msg`No, Foxhound's, kept as written`,
};

/** The scope a Limit is on: its kind's name and which one. */
export function Scope({ scope }: { readonly scope: LimitScope }): ReactNode {
  const id = scope.id;
  switch (scope.kind) {
    case "install":
      return <Trans>Install</Trans>;
    case "daemon":
      return (
        <Trans>
          Daemon <span translate="no">{id}</span>
        </Trans>
      );
    case "person":
      return (
        <Trans>
          Person <span translate="no">{id}</span>
        </Trans>
      );
    case "token":
      return (
        <Trans>
          Named token <span translate="no">{id}</span>
        </Trans>
      );
    case "connection":
      return (
        <Trans>
          Client connection <span translate="no">{id}</span>
        </Trans>
      );
    default: {
      const kind = scope.kind;
      return (
        <Trans>
          <span translate="no">{kind}</span> <span translate="no">{id}</span>
        </Trans>
      );
    }
  }
}

/** The Usage kind a Limit caps, in its currency for a price Limit. */
export function UsageKind({
  row,
}: {
  readonly row: Pick<LimitRow, "usageKind" | "currency">;
}): ReactNode {
  const kind = row.usageKind;
  const currency = row.currency;
  if (currency === undefined || currency === null) {
    return <code translate="no">{kind}</code>;
  }
  return (
    <Trans>
      <code translate="no">{kind}</code> in <span translate="no">{currency}</span>
    </Trans>
  );
}

/** When the period starts again, shown on the browser's clock. */
function Reset({ at }: { readonly at: PointInTime }): ReactNode {
  return (
    <time dateTime={at} translate="no">
      {formatInstant(at)}
    </time>
  );
}

/** A period's length in its time zone. */
const EVERY: Readonly<Record<Period["every"], (zone: ReactNode) => ReactNode>> = {
  day: (zone) => <Trans>Each day in {zone}</Trans>,
  week: (zone) => <Trans>Each week in {zone}</Trans>,
  month: (zone) => <Trans>Each month in {zone}</Trans>,
};

/** A period's length in its time zone, and when it starts again. */
const EVERY_UNTIL: Readonly<
  Record<Period["every"], (zone: ReactNode, reset: ReactNode) => ReactNode>
> = {
  day: (zone, reset) => (
    <Trans>
      Each day in {zone}, starting again {reset}
    </Trans>
  ),
  week: (zone, reset) => (
    <Trans>
      Each week in {zone}, starting again {reset}
    </Trans>
  ),
  month: (zone, reset) => (
    <Trans>
      Each month in {zone}, starting again {reset}
    </Trans>
  ),
};

/** A Limit's period in its time zone, and when it starts again. */
export function PeriodOf({
  period,
  resetsAt,
}: {
  readonly period: Period | null | undefined;
  readonly resetsAt: PointInTime | null | undefined;
}): ReactNode {
  if (period === undefined || period === null) {
    return <Trans>No period</Trans>;
  }
  const zone = <span translate="no">{period.time_zone}</span>;
  return resetsAt === undefined || resetsAt === null
    ? EVERY[period.every](zone)
    : EVERY_UNTIL[period.every](zone, <Reset at={resetsAt} />);
}

/** An amount of `row`'s Usage kind, in its machine form beside its formatted one. */
function Amount({ row, amount }: { readonly row: LimitRow; readonly amount: number }): ReactNode {
  return (
    <data value={String(amount)} translate="no">
      {formatAmount(amount, row.currency)}
    </data>
  );
}

/** Every Limit of `rows`, by scope, under `caption`; the ledger's columns when its rows carry them. */
export function LimitsTable({
  rows,
  caption,
}: {
  readonly rows: readonly LimitRow[];
  readonly caption: ReactNode;
}): ReactNode {
  const { i18n } = useLingui();
  const ledger = rows.some((row) => row.ledger !== undefined);
  return (
    <table className="w-full border-collapse text-start text-sm">
      <caption className="sr-only">{caption}</caption>
      <thead className="border-b">
        <tr>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Scope</Trans>
          </th>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Usage kind</Trans>
          </th>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Period</Trans>
          </th>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Limit</Trans>
          </th>
          {ledger ? (
            <>
              <th scope="col" className="py-2 pe-4 font-medium">
                <Trans>Charged</Trans>
              </th>
              <th scope="col" className="py-2 pe-4 font-medium">
                <Trans>Held by live runs</Trans>
              </th>
            </>
          ) : null}
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Left</Trans>
          </th>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Set by</Trans>
          </th>
          {ledger ? (
            <th scope="col" className="py-2 font-medium">
              <Trans>Must understand</Trans>
            </th>
          ) : null}
        </tr>
      </thead>
      <tbody>
        {byScope(rows).map((row, index) => (
          // A ledger may hold two rows alike but for their setter; their order is stable.
          <tr key={index} className="border-b align-top">
            <th scope="row" className="py-2 pe-4 font-normal">
              <Scope scope={row.scope} />
            </th>
            <td className="py-2 pe-4">
              <UsageKind row={row} />
            </td>
            <td className="py-2 pe-4">
              <PeriodOf period={row.period} resetsAt={row.resetsAt} />
            </td>
            <td className="py-2 pe-4 tabular-nums">
              <Amount row={row} amount={row.amount} />
            </td>
            {ledger ? (
              <>
                <td className="py-2 pe-4 tabular-nums">
                  {row.ledger === undefined ? null : (
                    <Amount row={row} amount={row.ledger.charged} />
                  )}
                </td>
                <td className="py-2 pe-4 tabular-nums">
                  {row.ledger === undefined ? null : <Amount row={row} amount={row.ledger.held} />}
                </td>
              </>
            ) : null}
            <td className="py-2 pe-4 tabular-nums">
              <Amount row={row} amount={row.left} />
            </td>
            <td className="py-2 pe-4">{i18n._(SETTERS[row.setBy])}</td>
            {ledger ? (
              <td className="py-2">
                {row.ledger === undefined
                  ? null
                  : i18n._(UNDERSTANDING[`${row.ledger.mustUnderstand}-${row.ledger.standing}`])}
              </td>
            ) : null}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
