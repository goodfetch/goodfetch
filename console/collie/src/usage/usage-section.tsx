// Usage and Limits, a section of Engine & settings (spec #591, story 99;
// ADR 0041): every Limit of the user's Usage ledger, by scope, with the
// Usage charged and held against it and what is left, read with
// `usage.list` from the engine the Console reached. Raising a Limit is a
// later ticket's (story 100).
import type { LimitUsage, UsageListResult } from "@goodfetch/collie-client/generated";
import { Trans } from "@lingui/react/macro";
import { useEffect, useId, useState, useSyncExternalStore, type ReactNode } from "react";
import type { ConsoleClient } from "#client/client.ts";
import { ErrorMessage } from "#errors/error-message.tsx";
import { LimitsTable, type LimitRow } from "./limits.tsx";

/** Where the reading of the ledger stands. */
export type UsageRead =
  | { readonly kind: "reading" }
  | { readonly kind: "read"; readonly usage: UsageListResult }
  /** The engine refused, with this error. */
  | { readonly kind: "failed"; readonly error: unknown };

/** A ledger row as the table shows it. */
export function rowOf(usage: LimitUsage): LimitRow {
  const { limit } = usage;
  return {
    scope: limit.scope,
    usageKind: limit.usage_kind,
    currency: limit.currency,
    period: limit.period,
    resetsAt: usage.resets_at,
    amount: limit.amount,
    left: usage.left,
    setBy: limit.set_by,
    ledger: {
      charged: usage.charged,
      held: usage.held,
      mustUnderstand: limit.must_understand,
      standing: usage.standing,
    },
  };
}

/** Where the reading stands, shown. */
function Read({ read }: { readonly read: UsageRead }): ReactNode {
  if (read.kind === "failed") {
    return (
      <div role="alert">
        <p>
          <Trans>The engine could not read the Usage ledger:</Trans>
        </p>
        <ErrorMessage error={read.error} />
      </div>
    );
  }
  if (read.kind === "reading") {
    return (
      <output className="block">
        <Trans>Reading the Usage ledger…</Trans>
      </output>
    );
  }
  if (read.usage.limits.length === 0) {
    return (
      <p>
        <Trans>No Limit holds your runs.</Trans>
      </p>
    );
  }
  return (
    <LimitsTable
      rows={read.usage.limits.map(rowOf)}
      caption={<Trans>Usage against each Limit</Trans>}
    />
  );
}

/** The Usage and Limits section, as it shows `read`. */
export function UsageView({ read }: { readonly read: UsageRead }): ReactNode {
  const heading = useId();
  return (
    <section aria-labelledby={heading} className="flex flex-col gap-4">
      <h2 id={heading} className="text-xl font-semibold">
        <Trans>Usage and Limits</Trans>
      </h2>
      <p className="text-muted-foreground">
        <Trans>
          The Usage counted against each Limit of your Usage ledger in its current period, by scope,
          as <code translate="no">goodfetch-collie usage list</code> shows it. A run that meets a
          Limit with nothing left stops at its next safe point.
        </Trans>
      </p>
      <Read read={read} />
    </section>
  );
}

/** The Usage and Limits section over `client`, read each time the Client connects. */
export function Usage({ client }: { readonly client: ConsoleClient }): ReactNode {
  const state = useSyncExternalStore(
    (changed) => client.watch(changed),
    () => client.state,
  );
  const [read, setRead] = useState<UsageRead>({ kind: "reading" });

  useEffect(() => {
    if (state !== "connected") {
      return undefined;
    }
    let current = true;
    const list = async (): Promise<void> => {
      let next: UsageRead;
      try {
        next = { kind: "read", usage: await client.call("usage.list", {}) };
      } catch (error) {
        next = { kind: "failed", error };
      }
      if (current) {
        setRead(next);
      }
    };
    void list();
    return () => {
      current = false;
    };
  }, [client, state]);

  return <UsageView read={read} />;
}
