// The Console's first launch offers a monthly Limit for the install (spec
// #591, story 8; ADR 0041): while the Usage ledger holds no install Limit,
// the Overview offers to cap the install's paid spending each month, in
// dollars, over a month in the browser's time zone. Accepted, the Limit is
// set through `usage.raise`, the person-only path (ADR 0029), and shows in
// the ledger; declined, the browser remembers it, so the offer is not made
// on every visit.
import type { UsageRaiseParams } from "@goodfetch/collie-client/generated";
import { Trans } from "@lingui/react/macro";
import {
  useEffect,
  useId,
  useState,
  useSyncExternalStore,
  type ReactNode,
  type SubmitEvent,
} from "react";
import type { ConsoleClient } from "#client/client.ts";
import { ErrorMessage } from "#errors/error-message.tsx";
import { Form } from "#form.tsx";
import { formatAmount, parseAmount } from "#format.ts";

/** The currency the offered Limit caps spending in. */
const CURRENCY = "USD";
/** The Usage kind the offered Limit caps: paid solves, which spend money. */
const USAGE_KIND = "paid_solves";
/** Where the browser remembers a declined offer, per origin and so per major. */
const DECLINED = "goodfetch-console-install-limit-offer";

/** What remembers that the person declined the offer. */
export interface OfferMemory {
  readonly declined: () => boolean;
  readonly decline: () => void;
}

/** The browser's memory of a declined offer; one that cannot store forgets it. */
export const browserMemory: OfferMemory = {
  declined: () => {
    try {
      return localStorage.getItem(DECLINED) === "declined";
    } catch {
      return false;
    }
  },
  decline: () => {
    try {
      localStorage.setItem(DECLINED, "declined");
    } catch {
      // A browser that stores nothing makes the offer again next time.
    }
  },
};

/** What `usage.raise` takes to cap the install's spending at `amount` millionths of dollars a month. */
export function monthlyInstallLimit(amount: number, timeZone: string): UsageRaiseParams {
  return {
    scope: "install",
    usage_kind: USAGE_KIND,
    currency: CURRENCY,
    period: { every: "month", time_zone: timeZone },
    amount,
  };
}

/** Where the offer stands. */
type Offer =
  | { readonly kind: "reading" }
  | { readonly kind: "offered" }
  | { readonly kind: "set"; readonly amount: number }
  | { readonly kind: "failed"; readonly error: unknown }
  | { readonly kind: "none" };

/** The first launch's offer of a monthly install Limit over `client`. */
export function InstallLimitOffer({
  client,
  memory = browserMemory,
}: {
  readonly client: ConsoleClient;
  readonly memory?: OfferMemory | undefined;
}): ReactNode {
  const state = useSyncExternalStore(
    (changed) => client.watch(changed),
    () => client.state,
  );
  const [offer, setOffer] = useState<Offer>({ kind: "reading" });
  const reading = offer.kind === "reading";

  useEffect(() => {
    if (state !== "connected" || !reading) {
      return undefined;
    }
    if (memory.declined()) {
      setOffer({ kind: "none" });
      return undefined;
    }
    let current = true;
    const read = async (): Promise<void> => {
      let next: Offer;
      try {
        const { limits } = await client.call("usage.list", {});
        const held = limits.some(({ limit }) => limit.scope.kind === "install");
        next = { kind: held ? "none" : "offered" };
      } catch {
        // Usage and Limits says why the ledger cannot be read.
        next = { kind: "none" };
      }
      if (current) {
        setOffer(next);
      }
    };
    void read();
    return () => {
      current = false;
    };
  }, [client, memory, reading, state]);

  const accept = async (amount: number): Promise<void> => {
    const zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
    try {
      await client.call("usage.raise", monthlyInstallLimit(amount, zone));
      setOffer({ kind: "set", amount });
    } catch (error) {
      setOffer({ kind: "failed", error });
    }
  };
  const decline = (): void => {
    memory.decline();
    setOffer({ kind: "none" });
  };
  return (
    <InstallLimitOfferView
      offer={offer}
      readOnly={state !== "connected"}
      onAccept={(amount) => void accept(amount)}
      onDecline={decline}
    />
  );
}

const buttonClassName =
  "self-start rounded-md border px-3 py-1.5 font-medium focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50";

/** The offer as it stands. */
function InstallLimitOfferView({
  offer,
  readOnly,
  onAccept,
  onDecline,
}: {
  readonly offer: Offer;
  readonly readOnly: boolean;
  readonly onAccept: (amount: number) => void;
  readonly onDecline: () => void;
}): ReactNode {
  const id = useId();
  const [refused, setRefused] = useState(false);
  if (offer.kind === "reading" || offer.kind === "none") {
    return null;
  }
  if (offer.kind === "set") {
    const shown = formatAmount(offer.amount, CURRENCY);
    return (
      <output className="block">
        <Trans>
          This install's spending is capped at{" "}
          <data value={String(offer.amount)} translate="no">
            {shown}
          </data>{" "}
          a month.
        </Trans>
      </output>
    );
  }
  const submit = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const typed = new FormData(event.currentTarget).get("amount");
    const parsed = typeof typed === "string" ? parseAmount(typed, CURRENCY) : undefined;
    if (parsed === undefined || parsed === 0) {
      setRefused(true);
      return;
    }
    setRefused(false);
    onAccept(parsed);
  };
  return (
    <section
      aria-labelledby={`${id}-heading`}
      className="flex flex-col gap-3 rounded-md border p-4"
    >
      <h2 id={`${id}-heading`} className="font-semibold">
        <Trans>Cap this install's spending</Trans>
      </h2>
      <p>
        <Trans>
          No Limit caps what this install's runs spend. Set a monthly Limit on paid solves, so
          spending is capped from today: a run that reaches it stops at its next safe point.
        </Trans>
      </p>
      <Form onSubmit={submit} className="flex flex-wrap items-end gap-3">
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-amount`} className="text-sm font-medium">
            <Trans>
              Each month, at most (<span translate="no">{CURRENCY}</span>)
            </Trans>
          </label>
          <input
            id={`${id}-amount`}
            name="amount"
            type="text"
            inputMode="decimal"
            autoComplete="off"
            required
            defaultValue="10"
            aria-invalid={refused}
            aria-describedby={refused ? `${id}-refused` : undefined}
            className="rounded-md border px-2 py-1.5 focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none"
          />
        </div>
        <button type="submit" disabled={readOnly} className={buttonClassName}>
          <Trans>Set the monthly Limit</Trans>
        </button>
        <button type="button" onClick={onDecline} className={buttonClassName}>
          <Trans>Not now</Trans>
        </button>
      </Form>
      {refused ? (
        <p id={`${id}-refused`} role="alert" className="text-destructive">
          <Trans>
            Enter an amount above zero in <span translate="no">{CURRENCY}</span>, such as 12.50.
          </Trans>
        </p>
      ) : null}
      {offer.kind === "failed" ? (
        <div role="alert">
          <p>
            <Trans>The engine did not set the Limit:</Trans>
          </p>
          <ErrorMessage error={offer.error} />
        </div>
      ) : null}
    </section>
  );
}
