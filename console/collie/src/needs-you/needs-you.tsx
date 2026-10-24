// "Needs you" (spec #591, stories 23, 25, 26, 32 and 100; ADRs 0029, 0031,
// 0041, 0048, 0052 and 0122): every item waiting for a person, the longest waiting first,
// each shown by its kind's renderer, a kind this build has none for shown by
// the engine's summary. Collie's storage kinds: a filesystem at the `low` or
// `critical` storage level with its folder, and a run stopped by storage,
// which a person resumes once space is freed or dismisses. A run stopped by
// a Limit that has nothing left offers to raise it, a person's answer
// (`usage.raise`, which only the Console and the interactive CLI prompt
// offer, so an agent a Limit stopped cannot lift it): once raised, a
// Recipe's run resumes in the Daemon (`run.resume`), and an SDK script's
// run waits for its script, its resume command offered to copy. A domain
// that showed a consent wall is offered for `consent.domains`, alone or as
// `*`, each offer beside the plain warning of what the person takes on and
// answered in one click (`needs_you.allow_consent_domain`, a person's
// answer), with no confirmation asked. Items leave the list as they are
// resolved, here, elsewhere or by their run.
import type {
  LimitScope,
  NeedsYouItem,
  Period,
  PointInTime,
  Product,
  ReachedLimit,
  UsageRaiseParams,
} from "@goodfetch/collie-client/generated";
import { Trans, useLingui } from "@lingui/react/macro";
import { useId, useState, type ReactNode, type SubmitEvent } from "react";
import type { ConsoleClient } from "#client/client.ts";
import { formatAmount, formatInstant, parseAmount } from "#format.ts";
import { resumeCommand } from "#runs/run-page.tsx";
import { PeriodOf, Scope, UsageKind } from "#usage/limits.tsx";
import type { NeedsYouState } from "./needs-you-store.ts";

/** A person's answer to a run stopped by a Limit, once the engine took it. */
export type Raised =
  | {
      /** The Recipe's run resumes in the Daemon. */
      readonly kind: "resumed";
      readonly run: string;
    }
  | {
      /** The SDK script's run waits for its script, which `command` resumes. */
      readonly kind: "waiting_for_script";
      readonly run: string;
      readonly command: string;
    }
  | {
      /** The Limit is raised, but the engine did not resume the Recipe's run. */
      readonly kind: "not_resumed";
      readonly run: string;
      readonly message: string;
    };

/** What became of a copy of a resume command. */
export type Copied = "copied" | "copy_refused";

/** What "Needs you" shows. */
export interface NeedsYouViewProps {
  readonly state: NeedsYouState;
  /** Whether the Console has no Connection now, which disables every action. */
  readonly readOnly?: boolean | undefined;
  /** Dismisses the item `id`. */
  readonly onDismiss: (id: string) => void;
  /** Raises the Limit that stopped the run of `item` to `amount`. */
  readonly onRaise?: ((item: NeedsYouItem, stop: LimitStop, amount: number) => void) | undefined;
  /** The answers to runs stopped by a Limit the engine took, the latest last. */
  readonly raised?: readonly Raised[] | undefined;
  /** Copies `command`. */
  readonly onCopy?: ((command: string) => void) | undefined;
  /** What became of the last copy. */
  readonly copied?: Copied | undefined;
  /** Adds `domain`, or `*`, to `consent.domains` for the next runs. */
  readonly onAllowConsentDomain?: ((domain: string) => void) | undefined;
}

/** The storage level a `storage_level` item names, and its folder. */
interface StorageLevelDetails {
  readonly level: "low" | "critical";
  readonly folder: string;
}

/** The folder a `run_stopped_by_storage` item names. */
interface StoppedDetails {
  readonly folder: string;
}

/** The domain a `consent_wall` item names. */
interface ConsentWallDetails {
  readonly domain: string;
}

/** What a `run_stopped_by_limit` item names: the run, the Limit that stopped it, what it allows now, the Product the run needs and, for a Recipe's run, its Recipe. */
export interface LimitStop {
  readonly run: string;
  readonly limit: ReachedLimit;
  readonly amount: number;
  readonly needs: Product;
  readonly recipe: string | undefined;
}

/** The details of a `storage_level` item, if they hold a level and a folder. */
function storageLevelOf(item: NeedsYouItem): StorageLevelDetails | undefined {
  const { level, folder } = item.details ?? {};
  if ((level === "low" || level === "critical") && typeof folder === "string") {
    return { level, folder };
  }
  return undefined;
}

/** The details of a `run_stopped_by_storage` item, if they hold a folder. */
function stoppedOf(item: NeedsYouItem): StoppedDetails | undefined {
  const { folder } = item.details ?? {};
  return typeof folder === "string" ? { folder } : undefined;
}

/** The details of a `consent_wall` item, if they hold a domain. */
function consentWallOf(item: NeedsYouItem): ConsentWallDetails | undefined {
  const { domain } = item.details ?? {};
  return typeof domain === "string" ? { domain } : undefined;
}

/** The member `name` of `value`, if it is an object. */
function member(value: unknown, name: string): unknown {
  return typeof value === "object" && value !== null
    ? Object.entries(value).find(([key]) => key === name)?.[1]
    : undefined;
}

/** `value` as an optional string: absent, `null` or a string; `false` for anything else. */
function optionalString(value: unknown): string | undefined | false {
  if (value === undefined || value === null) {
    return undefined;
  }
  return typeof value === "string" ? value : false;
}

/** `value` as a Limit's scope, if it is one. */
function scopeOf(value: unknown): LimitScope | undefined {
  const kind = member(value, "kind");
  const id = member(value, "id");
  return typeof kind === "string" && typeof id === "string" ? { kind, id } : undefined;
}

/** `value` as a Limit's period, absent, or `false` when it is neither. */
function periodOf(value: unknown): Period | undefined | false {
  if (value === undefined || value === null) {
    return undefined;
  }
  const every = member(value, "every");
  const zone = member(value, "time_zone");
  return (every === "day" || every === "week" || every === "month") && typeof zone === "string"
    ? { every, time_zone: zone }
    : false;
}

/** `value` as the Limit `limit_reached` names, if it is one. */
function reachedOf(value: unknown): ReachedLimit | undefined {
  const scope = scopeOf(member(value, "scope"));
  const usageKind = member(value, "usage_kind");
  const currency = optionalString(member(value, "currency"));
  const period = periodOf(member(value, "period"));
  const resetsAt: PointInTime | undefined | false = optionalString(member(value, "resets_at"));
  if (
    scope === undefined ||
    typeof usageKind !== "string" ||
    currency === false ||
    period === false ||
    resetsAt === false
  ) {
    return undefined;
  }
  return {
    scope,
    usage_kind: usageKind,
    ...(currency === undefined ? {} : { currency }),
    ...(period === undefined ? {} : { period }),
    ...(resetsAt === undefined ? {} : { resets_at: resetsAt }),
  };
}

/** The details of a `run_stopped_by_limit` item, if they hold its run, its Limit, what it allows and the Product the run needs. */
export function limitStopOf(item: NeedsYouItem): LimitStop | undefined {
  const { limit, amount, needs, recipe } = item.details ?? {};
  const reached = reachedOf(limit);
  const named = optionalString(recipe);
  if (
    typeof item.run !== "string" ||
    reached === undefined ||
    typeof amount !== "number" ||
    (needs !== "collie" && needs !== "foxhound") ||
    named === false
  ) {
    return undefined;
  }
  return { run: item.run, limit: reached, amount, needs, recipe: named };
}

/** What `usage.raise` takes to raise `limit` to `amount`. */
export function raiseParams(limit: ReachedLimit, amount: number): UsageRaiseParams {
  return {
    scope: limit.scope.kind,
    scope_id: limit.scope.id,
    usage_kind: limit.usage_kind,
    ...(limit.currency === undefined || limit.currency === null
      ? {}
      : { currency: limit.currency }),
    ...(limit.period === undefined || limit.period === null ? {} : { period: limit.period }),
    amount,
  };
}

/** A filesystem at a storage level. */
function StorageLevel({ details }: { readonly details: StorageLevelDetails }): ReactNode {
  const folder = details.folder;
  return details.level === "critical" ? (
    <>
      <h2 className="font-semibold">
        <Trans>A filesystem is at the critical storage level</Trans>
      </h2>
      <p>
        <Trans>
          The filesystem holding <code translate="no">{folder}</code> is at the critical storage
          level: the runs writing there stopped. Free space on it, then start each again with its
          Run ID.
        </Trans>
      </p>
    </>
  ) : (
    <>
      <h2 className="font-semibold">
        <Trans>A filesystem is at the low storage level</Trans>
      </h2>
      <p>
        <Trans>
          The filesystem holding <code translate="no">{folder}</code> is at the low storage level:
          its runs go on, but shed what they can. Free space on it.
        </Trans>
      </p>
    </>
  );
}

/** A run stopped by storage. */
function Stopped({
  run,
  details,
}: {
  readonly run: string;
  readonly details: StoppedDetails;
}): ReactNode {
  const folder = details.folder;
  return (
    <>
      <h2 className="font-semibold">
        <Trans>
          The run <code translate="no">{run}</code> stopped: its disk filled
        </Trans>
      </h2>
      <p>
        <Trans>
          The filesystem holding <code translate="no">{folder}</code> filled. Free space on it, then
          start the run again with its Run ID to resume it.
        </Trans>
      </p>
    </>
  );
}

const buttonClassName =
  "self-start rounded-md border px-3 py-1.5 font-medium focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50";

/** Why a typed amount is refused, before it reaches the engine. */
type Refused = "not_an_amount" | "not_above";

/** A run stopped by a Limit, with the form that raises it. */
function StoppedByLimit({
  stop,
  readOnly,
  onRaise,
}: {
  readonly stop: LimitStop;
  readonly readOnly: boolean;
  readonly onRaise: ((amount: number) => void) | undefined;
}): ReactNode {
  const id = useId();
  const [refused, setRefused] = useState<Refused | undefined>(undefined);
  const { run, recipe, limit, amount } = stop;
  const now = formatAmount(amount, limit.currency);
  const currency = limit.currency ?? undefined;
  const submit = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const typed = new FormData(event.currentTarget).get("amount");
    const parsed = typeof typed === "string" ? parseAmount(typed, currency) : undefined;
    if (parsed === undefined) {
      setRefused("not_an_amount");
    } else if (parsed <= amount) {
      setRefused("not_above");
    } else {
      setRefused(undefined);
      onRaise?.(parsed);
    }
  };
  return (
    <>
      <h2 className="font-semibold">
        {recipe === undefined ? (
          <Trans>
            The run <code translate="no">{run}</code> stopped: a Limit has nothing left
          </Trans>
        ) : (
          <Trans>
            The run <code translate="no">{run}</code> of the Recipe{" "}
            <code translate="no">{recipe}</code> stopped: a Limit has nothing left
          </Trans>
        )}
      </h2>
      <p>
        <Trans>
          <Scope scope={limit.scope} />, on{" "}
          <UsageKind row={{ usageKind: limit.usage_kind, currency: limit.currency }} />:{" "}
          <PeriodOf period={limit.period} resetsAt={limit.resets_at} />. It allows{" "}
          <data value={String(amount)} translate="no">
            {now}
          </data>
          , and nothing is left.
        </Trans>
      </p>
      <p>
        {recipe === undefined ? (
          <Trans>
            Raise it to let the run go on: it then waits for its script, which resumes it when
            started again with its Run ID.
          </Trans>
        ) : (
          <Trans>Raise it to let the run go on: the Daemon then resumes it.</Trans>
        )}
      </p>
      <form onSubmit={submit} className="flex flex-wrap items-end gap-3">
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-amount`} className="text-sm font-medium">
            {limit.period === undefined || limit.period === null ? (
              <Trans>New amount</Trans>
            ) : (
              <Trans>New amount per period</Trans>
            )}
          </label>
          <input
            id={`${id}-amount`}
            name="amount"
            type="text"
            inputMode={currency === undefined ? "numeric" : "decimal"}
            autoComplete="off"
            required
            aria-invalid={refused !== undefined}
            aria-describedby={refused === undefined ? undefined : `${id}-refused`}
            className="rounded-md border px-2 py-1.5 focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none"
          />
        </div>
        <button type="submit" disabled={readOnly} className={buttonClassName}>
          <Trans>Raise the Limit</Trans>
        </button>
      </form>
      {refused === undefined ? null : (
        <p id={`${id}-refused`} role="alert" className="text-destructive">
          {refused === "not_above" ? (
            <Trans>Enter an amount above the {now} it allows now.</Trans>
          ) : currency === undefined ? (
            <Trans>Enter the new amount as a whole number.</Trans>
          ) : (
            <Trans>
              Enter the new amount in <span translate="no">{currency}</span>, such as 12.50.
            </Trans>
          )}
        </p>
      )}
    </>
  );
}

/** A domain that showed a consent wall, with its two offers, each beside what it takes on. */
function ConsentWall({
  details,
  readOnly,
  onAllow,
}: {
  readonly details: ConsentWallDetails;
  readonly readOnly: boolean;
  readonly onAllow: ((domain: string) => void) | undefined;
}): ReactNode {
  const domain = details.domain;
  return (
    <>
      <h2 className="font-semibold">
        <Trans>
          <span translate="no">{domain}</span> showed a consent wall
        </Trans>
      </h2>
      <p>
        <Trans>
          No run accepts a consent wall unless a person approved its domain, so its pages stayed at{" "}
          <code translate="no">consent_required</code>. You may add{" "}
          <span translate="no">{domain}</span> to <code translate="no">consent.domains</code> in
          your configuration for the next runs.
        </Trans>
      </p>
      <p className="text-sm">
        <Trans>
          Accepting the consent walls of <span translate="no">{domain}</span> gives consent in your
          name: the site may load its trackers and set their cookies, and under the GDPR that
          consent is yours. Every later run accepts them until you remove{" "}
          <span translate="no">{domain}</span> from <code translate="no">consent.domains</code>.
        </Trans>
      </p>
      <button
        type="button"
        disabled={readOnly}
        onClick={() => onAllow?.(domain)}
        className={buttonClassName}
      >
        <Trans>Accept {domain}'s consent walls in later runs</Trans>
      </button>
      <p className="text-sm">
        <Trans>
          Accepting consent walls on every domain gives consent in your name on every site that
          shows one: those sites may load their trackers and set their cookies, and under the GDPR
          that consent is yours. An Agent also no longer asks you before accepting a consent prompt
          on any domain. Every later run accepts them until you remove <code translate="no">*</code>{" "}
          from <code translate="no">consent.domains</code>.
        </Trans>
      </p>
      <button
        type="button"
        disabled={readOnly}
        onClick={() => onAllow?.("*")}
        className={buttonClassName}
      >
        <Trans>Accept consent walls on every domain</Trans>
      </button>
    </>
  );
}

/** What an item says, by its kind's renderer, or by the engine's summary for a kind this build has none for. */
function Content({
  item,
  readOnly,
  onRaise,
  onAllowConsentDomain,
}: {
  readonly item: NeedsYouItem;
  readonly readOnly: boolean;
  readonly onRaise: NeedsYouViewProps["onRaise"];
  readonly onAllowConsentDomain: NeedsYouViewProps["onAllowConsentDomain"];
}): ReactNode {
  if (item.kind === "consent_wall" && item.actions.includes("needs_you.allow_consent_domain")) {
    const details = consentWallOf(item);
    if (details !== undefined) {
      return <ConsentWall details={details} readOnly={readOnly} onAllow={onAllowConsentDomain} />;
    }
  }
  if (item.kind === "storage_level") {
    const details = storageLevelOf(item);
    if (details !== undefined) {
      return <StorageLevel details={details} />;
    }
  }
  if (item.kind === "run_stopped_by_storage" && typeof item.run === "string") {
    const details = stoppedOf(item);
    if (details !== undefined) {
      return <Stopped run={item.run} details={details} />;
    }
  }
  if (item.kind === "run_stopped_by_limit" && item.actions.includes("usage.raise")) {
    const stop = limitStopOf(item);
    if (stop !== undefined) {
      return (
        <StoppedByLimit
          stop={stop}
          readOnly={readOnly}
          onRaise={onRaise === undefined ? undefined : (amount) => onRaise(item, stop, amount)}
        />
      );
    }
  }
  return <p translate="no">{item.summary}</p>;
}

/** One item, with since when it waits and the actions it offers. */
function Item({
  item,
  readOnly,
  onDismiss,
  onRaise,
  onAllowConsentDomain,
}: {
  readonly item: NeedsYouItem;
  readonly readOnly: boolean;
  readonly onDismiss: (id: string) => void;
  readonly onRaise: NeedsYouViewProps["onRaise"];
  readonly onAllowConsentDomain: NeedsYouViewProps["onAllowConsentDomain"];
}): ReactNode {
  const { i18n } = useLingui();
  const since = formatInstant(item.since, i18n.locale);
  return (
    <li className="flex flex-col gap-2 rounded-lg border p-4">
      <Content
        item={item}
        readOnly={readOnly}
        onRaise={onRaise}
        onAllowConsentDomain={onAllowConsentDomain}
      />
      <p className="text-muted-foreground text-sm">
        <Trans>
          Waiting since <time dateTime={item.since}>{since}</time>
        </Trans>
      </p>
      {item.actions.includes("needs_you.dismiss") ? (
        <button
          type="button"
          disabled={readOnly}
          onClick={() => onDismiss(item.id)}
          className={buttonClassName}
        >
          <Trans>Dismiss</Trans>
        </button>
      ) : null}
    </li>
  );
}

/** A person's answer to a run stopped by a Limit, as the engine took it. */
function RaisedNotice({
  raised,
  onCopy,
}: {
  readonly raised: Raised;
  readonly onCopy: NeedsYouViewProps["onCopy"];
}): ReactNode {
  const run = raised.run;
  switch (raised.kind) {
    case "resumed":
      return (
        <p role="status">
          <Trans>
            You raised the Limit, and the run <code translate="no">{run}</code> resumes in the
            Daemon.
          </Trans>
        </p>
      );
    case "not_resumed": {
      const message = raised.message;
      return (
        <p role="alert" className="text-destructive">
          <Trans>
            You raised the Limit, but the engine did not resume the run{" "}
            <code translate="no">{run}</code>: <span translate="no">{message}</span>
          </Trans>
        </p>
      );
    }
    case "waiting_for_script": {
      const command = raised.command;
      return (
        <div className="flex flex-col gap-2">
          <p role="status">
            <Trans>
              You raised the Limit. The run <code translate="no">{run}</code> waits for its script:
              start the program again with its Run ID to resume it.
            </Trans>
          </p>
          <code translate="no" className="self-start rounded-md bg-muted px-2 py-1 font-mono">
            {command}
          </code>
          <button type="button" onClick={() => onCopy?.(command)} className={buttonClassName}>
            <Trans>Copy resume command</Trans>
          </button>
        </div>
      );
    }
    default:
      return raised satisfies never;
  }
}

/** What became of the last copy. */
function CopiedNotice({ copied }: { readonly copied: Copied | undefined }): ReactNode {
  return (
    <p role="status">
      {copied === "copied" ? (
        <Trans>Copied the resume command.</Trans>
      ) : copied === "copy_refused" ? (
        <Trans>The browser refused to copy the resume command: select it and copy it.</Trans>
      ) : null}
    </p>
  );
}

/** "Needs you", as it shows `state`. */
export function NeedsYouView({
  state,
  readOnly = false,
  onDismiss,
  onRaise,
  raised = [],
  onCopy,
  copied,
  onAllowConsentDomain,
}: NeedsYouViewProps): ReactNode {
  const { t } = useLingui();
  return (
    <div className="flex flex-col gap-6">
      <h1 className="font-heading text-3xl font-semibold tracking-tight">
        <Trans>Needs you</Trans>
      </h1>
      {raised.length === 0 ? null : (
        <section aria-label={t`Your answers`} className="flex flex-col gap-3">
          {raised.map((answer, index) => (
            // The answers only grow, so an answer's place names it.
            <RaisedNotice key={index} raised={answer} onCopy={onCopy} />
          ))}
          <CopiedNotice copied={copied} />
        </section>
      )}
      {state.kind === "reading" ? (
        <p role="status">
          <Trans>Reading what needs you…</Trans>
        </p>
      ) : state.kind === "failed" ? (
        <FailedList message={state.message} />
      ) : state.items.length === 0 ? (
        <p className="text-muted-foreground">
          <Trans>Nothing needs you now.</Trans>
        </p>
      ) : (
        <ul aria-label={t`Needs you`} className="flex flex-col gap-3">
          {state.items.map((item) => (
            <Item
              key={item.id}
              item={item}
              readOnly={readOnly}
              onDismiss={onDismiss}
              onRaise={onRaise}
              onAllowConsentDomain={onAllowConsentDomain}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

/** The engine's refusal of the list. */
function FailedList({ message }: { readonly message: string }): ReactNode {
  return (
    <p role="alert">
      <Trans>
        The engine could not list what needs you: <span translate="no">{message}</span>
      </Trans>
    </p>
  );
}

/** What the engine refused, and the message it gave. */
type Failure = {
  readonly kind: "dismiss" | "raise" | "allow_consent_domain";
  readonly message: string;
};

/** The message of `error`, a refusal or any other failure. */
function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** "Needs you" over the shell's list, dismissing through `dismiss`, and raising a run's Limit and answering a consent wall over `client`. */
export function NeedsYou({
  client,
  state,
  readOnly,
  dismiss,
}: {
  readonly client: ConsoleClient;
  readonly state: NeedsYouState;
  readonly readOnly: boolean;
  readonly dismiss: (id: string) => Promise<boolean>;
}): ReactNode {
  const [failure, setFailure] = useState<Failure | undefined>(undefined);
  const [raised, setRaised] = useState<readonly Raised[]>([]);
  const [copied, setCopied] = useState<Copied | undefined>(undefined);
  const onDismiss = (id: string): void => {
    setFailure(undefined);
    dismiss(id).catch((error: unknown) => {
      setFailure({ kind: "dismiss", message: messageOf(error) });
    });
  };
  const raise = async (item: NeedsYouItem, stop: LimitStop, amount: number): Promise<void> => {
    try {
      await client.call("usage.raise", raiseParams(stop.limit, amount));
    } catch (error) {
      setFailure({ kind: "raise", message: messageOf(error) });
      return;
    }
    let answer: Raised;
    if (item.actions.includes("run.resume")) {
      try {
        await client.call("run.resume", { run_id: stop.run });
        answer = { kind: "resumed", run: stop.run };
      } catch (error) {
        answer = { kind: "not_resumed", run: stop.run, message: messageOf(error) };
      }
    } else {
      answer = {
        kind: "waiting_for_script",
        run: stop.run,
        command: resumeCommand(stop.run, stop.needs),
      };
    }
    setRaised((before) => [...before, answer]);
  };
  const onRaise = (item: NeedsYouItem, stop: LimitStop, amount: number): void => {
    setFailure(undefined);
    void raise(item, stop, amount);
  };
  const onAllowConsentDomain = (domain: string): void => {
    setFailure(undefined);
    client.call("needs_you.allow_consent_domain", { domain }).catch((error: unknown) => {
      setFailure({ kind: "allow_consent_domain", message: messageOf(error) });
    });
  };
  const onCopy = (command: string): void => {
    navigator.clipboard.writeText(command).then(
      () => setCopied("copied"),
      () => setCopied("copy_refused"),
    );
  };
  return (
    <>
      <NeedsYouView
        state={state}
        readOnly={readOnly}
        onDismiss={onDismiss}
        onRaise={onRaise}
        raised={raised}
        onCopy={onCopy}
        copied={copied}
        onAllowConsentDomain={onAllowConsentDomain}
      />
      {failure === undefined ? null : (
        <p role="alert" className="text-destructive">
          <FailureText failure={failure} />
        </p>
      )}
    </>
  );
}

/** What the engine refused, in words. */
function FailureText({ failure }: { readonly failure: Failure }): ReactNode {
  const message = failure.message;
  switch (failure.kind) {
    case "dismiss":
      return (
        <Trans>
          The engine could not dismiss the item: <span translate="no">{message}</span>
        </Trans>
      );
    case "raise":
      return (
        <Trans>
          The engine did not raise the Limit: <span translate="no">{message}</span>
        </Trans>
      );
    case "allow_consent_domain":
      return (
        <Trans>
          The engine could not add the domain to <code translate="no">consent.domains</code>:{" "}
          <span translate="no">{message}</span>
        </Trans>
      );
    default:
      return failure.kind satisfies never;
  }
}
