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
// answer), with no confirmation asked. A domain on which a run's refetch
// without the site's stored data stopped (story 27, ADR 0052) is shown with
// the layer of `wall.refetch_domains` that lists it and its file: the person
// removes it there, replaces `*` with a list when only `*` covers it, or
// denies it in `wall.denied_domains` with a reason; a domain project
// configuration lists is changed in that file, never here. A certificate
// whose key changed (story 28, ADR 0042) shows both keys, answered by
// `certs.accept` or `certs.forget`, which act in the caller's own pins.
// Items leave the list as they are resolved, here, elsewhere or by their run.
import type {
  LimitScope,
  NeedsYouItem,
  Period,
  PointInTime,
  Product,
  ReachedLimit,
  SettingsLayer,
  UsageRaiseParams,
} from "@goodfetch/collie-client/generated";
import { Trans, useLingui } from "@lingui/react/macro";
import { useId, useState, type ReactNode, type SubmitEvent } from "react";
import type { ConsoleClient } from "#client/client.ts";
import { formatAmount, formatInstant, parseAmount } from "#format.ts";
import { ErrorMessage } from "#errors/error-message.tsx";
import { resumeCommand } from "#runs/run-page.tsx";
import { isSettingsLayer, LAYERS } from "#settings-layers.ts";
import { PeriodOf, Scope, UsageKind } from "#usage/limits.tsx";
import type { NeedsYouState } from "./needs-you-store.ts";
import { Form } from "#form.tsx";

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
      readonly error: unknown;
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
  /** Removes `domain` from the layer of `wall.refetch_domains` that lists it, or replaces `*` there with `insteadOfStar`. */
  readonly onRemoveRefetchDomain?:
    | ((domain: string, insteadOfStar: readonly string[] | undefined) => void)
    | undefined;
  /** Adds `domain` to `wall.denied_domains` with `reason`. */
  readonly onDenyWallDomain?: ((domain: string, reason: string) => void) | undefined;
  /** Pins `key`, the changed key `host` presented, in place of its pin. */
  readonly onAcceptCertificate?: ((host: string, key: string) => void) | undefined;
  /** Forgets the pin of `host`. */
  readonly onForgetCertificate?: ((host: string) => void) | undefined;
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

/** What a `wall_refetch_stopped` item names: the domain, what stopped the refetch, and the layer of `wall.refetch_domains` that lists it, with its file. */
interface RefetchStoppedDetails {
  readonly domain: string;
  readonly cause: string;
  readonly layer: SettingsLayer;
  readonly file: string;
  readonly onlyStar: boolean;
}

/** The details of a `wall_refetch_stopped` item, if they hold all of its fields. */
function refetchStoppedOf(item: NeedsYouItem): RefetchStoppedDetails | undefined {
  const { domain, cause, layer, file, only_star: onlyStar } = item.details ?? {};
  return typeof domain === "string" &&
    typeof cause === "string" &&
    typeof layer === "string" &&
    isSettingsLayer(layer) &&
    typeof file === "string" &&
    typeof onlyStar === "boolean"
    ? { domain, cause, layer, file, onlyStar }
    : undefined;
}

/** What a `certificate_changed` item names: the host, and its pinned and presented keys. */
interface ChangedCertificateDetails {
  readonly host: string;
  readonly pinned: string | undefined;
  readonly presented: string;
}

/** The details of a `certificate_changed` item, if they hold its host and keys. */
function changedCertificateOf(item: NeedsYouItem): ChangedCertificateDetails | undefined {
  const { host, pinned_spki_sha256: pinned, presented_spki_sha256: presented } = item.details ?? {};
  const pin = optionalString(pinned);
  return typeof host === "string" && typeof presented === "string" && pin !== false
    ? { host, pinned: pin, presented }
    : undefined;
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
      <Form onSubmit={submit} className="flex flex-wrap items-end gap-3">
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
      </Form>
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
        <Trans>
          Accept <span translate="no">{domain}</span>'s consent walls in later runs
        </Trans>
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

/** The domains a person types, separated by commas or spaces. */
function domainsTyped(typed: string): string[] {
  return typed
    .split(/[\s,]+/u)
    .map((domain) => domain.trim())
    .filter((domain) => domain !== "");
}

/** A domain on which a run's refetch without the site's stored data stopped, with the layer that lists it and the answers it offers. */
function RefetchStopped({
  details,
  removable,
  readOnly,
  onRemove,
  onDeny,
}: {
  readonly details: RefetchStoppedDetails;
  readonly removable: boolean;
  readonly readOnly: boolean;
  readonly onRemove: NeedsYouViewProps["onRemoveRefetchDomain"];
  readonly onDeny: NeedsYouViewProps["onDenyWallDomain"];
}): ReactNode {
  const id = useId();
  const { i18n } = useLingui();
  const { domain, cause, layer, file } = details;
  const layerName = i18n._(LAYERS[layer]);
  const replace = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const typed = new FormData(event.currentTarget).get("domains");
    onRemove?.(domain, domainsTyped(typeof typed === "string" ? typed : ""));
  };
  const deny = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const typed = new FormData(event.currentTarget).get("reason");
    onDeny?.(domain, typeof typed === "string" ? typed : "");
  };
  return (
    <>
      <h2 className="font-semibold">
        <Trans>
          <span translate="no">{domain}</span> refused a page fetched again without its stored data
        </Trans>
      </h2>
      <p>
        <Trans>
          A run fetched a page of <span translate="no">{domain}</span> again without the site's
          stored cookies and storage, and the site answered with <code translate="no">{cause}</code>
          . No page of the site was fetched that way again for the rest of that run.
        </Trans>
      </p>
      {layer === "project_configuration" ? (
        <p>
          <Trans>
            Project configuration lists <span translate="no">{domain}</span> in{" "}
            <code translate="no">wall.refetch_domains</code>, in <code translate="no">{file}</code>:
            remove it from that file to stop the next runs of that project from fetching its pages
            again.
          </Trans>
        </p>
      ) : details.onlyStar ? (
        <>
          <p>
            <Trans>
              <code translate="no">*</code> in <code translate="no">wall.refetch_domains</code>{" "}
              covers <span translate="no">{domain}</span>, in {layerName}:{" "}
              <code translate="no">{file}</code>. Give the domains that replace{" "}
              <code translate="no">*</code> there, leaving <span translate="no">{domain}</span> out.
            </Trans>
          </p>
          {removable ? (
            <Form onSubmit={replace} className="flex flex-wrap items-end gap-3">
              <div className="flex flex-col gap-1">
                <label htmlFor={`${id}-domains`} className="text-sm font-medium">
                  <Trans>Domains that replace *</Trans>
                </label>
                <input
                  id={`${id}-domains`}
                  name="domains"
                  type="text"
                  autoComplete="off"
                  required
                  className="rounded-md border px-2 py-1.5 focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none"
                />
              </div>
              <button type="submit" disabled={readOnly} className={buttonClassName}>
                <Trans>Replace * with this list</Trans>
              </button>
            </Form>
          ) : null}
        </>
      ) : (
        <>
          <p>
            <Trans>
              {layerName} lists <span translate="no">{domain}</span> in{" "}
              <code translate="no">wall.refetch_domains</code>, in{" "}
              <code translate="no">{file}</code>.
            </Trans>
          </p>
          {removable ? (
            <button
              type="button"
              disabled={readOnly}
              onClick={() => onRemove?.(domain, undefined)}
              className={buttonClassName}
            >
              <Trans>
                Remove <span translate="no">{domain}</span> from{" "}
                <span translate="no">wall.refetch_domains</span>
              </Trans>
            </button>
          ) : null}
        </>
      )}
      <p className="text-sm">
        <Trans>
          Denying <span translate="no">{domain}</span> adds it to{" "}
          <code translate="no">wall.denied_domains</code> in the Operator policy: no run reads past
          its Walls or fetches its pages again without its stored data, whatever another layer
          allows. The Audit log records your reason.
        </Trans>
      </p>
      <Form onSubmit={deny} className="flex flex-wrap items-end gap-3">
        <div className="flex flex-col gap-1">
          <label htmlFor={`${id}-reason`} className="text-sm font-medium">
            <Trans>Reason for the denial</Trans>
          </label>
          <input
            id={`${id}-reason`}
            name="reason"
            type="text"
            autoComplete="off"
            required
            className="rounded-md border px-2 py-1.5 focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none"
          />
        </div>
        <button type="submit" disabled={readOnly} className={buttonClassName}>
          <Trans>
            Deny <span translate="no">{domain}</span>
          </Trans>
        </button>
      </Form>
    </>
  );
}

/** A pinned host whose certificate's key changed, with the answers of the person whose store holds the pin. */
function ChangedCertificate({
  details,
  readOnly,
  onAccept,
  onForget,
}: {
  readonly details: ChangedCertificateDetails;
  readonly readOnly: boolean;
  readonly onAccept: NeedsYouViewProps["onAcceptCertificate"];
  readonly onForget: NeedsYouViewProps["onForgetCertificate"];
}): ReactNode {
  const { host, pinned, presented } = details;
  return (
    <>
      <h2 className="font-semibold">
        <Trans>
          The certificate of <span translate="no">{host}</span> changed its key
        </Trans>
      </h2>
      <p>
        <Trans>
          <span translate="no">{host}</span> presented a certificate whose public key is not the one
          you pinned, so nothing is sent to it. Accept the new key only if you know why it changed;
          forgetting the pin lets its next connection pin again.
        </Trans>
      </p>
      <dl className="grid grid-cols-[auto_1fr] gap-x-3 text-sm">
        <dt>
          <Trans>Pinned key</Trans>
        </dt>
        <dd>
          {pinned === undefined ? (
            <Trans>The pin cannot be read.</Trans>
          ) : (
            <code translate="no" className="break-all">
              {pinned}
            </code>
          )}
        </dd>
        <dt>
          <Trans>Presented key</Trans>
        </dt>
        <dd>
          <code translate="no" className="break-all">
            {presented}
          </code>
        </dd>
      </dl>
      <div className="flex flex-wrap gap-3">
        <button
          type="button"
          disabled={readOnly}
          onClick={() => onAccept?.(host, presented)}
          className={buttonClassName}
        >
          <Trans>Accept the new key</Trans>
        </button>
        <button
          type="button"
          disabled={readOnly}
          onClick={() => onForget?.(host)}
          className={buttonClassName}
        >
          <Trans>Forget the pin</Trans>
        </button>
      </div>
    </>
  );
}

/** The answers an item may need, as the view takes them. */
type Answers = Pick<
  NeedsYouViewProps,
  | "onRaise"
  | "onAllowConsentDomain"
  | "onRemoveRefetchDomain"
  | "onDenyWallDomain"
  | "onAcceptCertificate"
  | "onForgetCertificate"
>;

/** What an item says, by its kind's renderer, or by the engine's summary for a kind this build has none for. */
function Content({
  item,
  readOnly,
  answers,
}: {
  readonly item: NeedsYouItem;
  readonly readOnly: boolean;
  readonly answers: Answers;
}): ReactNode {
  const { onRaise, onAllowConsentDomain } = answers;
  if (item.kind === "wall_refetch_stopped" && item.actions.includes("needs_you.deny_wall_domain")) {
    const details = refetchStoppedOf(item);
    if (details !== undefined) {
      return (
        <RefetchStopped
          details={details}
          removable={item.actions.includes("needs_you.remove_refetch_domain")}
          readOnly={readOnly}
          onRemove={answers.onRemoveRefetchDomain}
          onDeny={answers.onDenyWallDomain}
        />
      );
    }
  }
  if (item.kind === "certificate_changed" && item.actions.includes("certs.accept")) {
    const details = changedCertificateOf(item);
    if (details !== undefined) {
      return (
        <ChangedCertificate
          details={details}
          readOnly={readOnly}
          onAccept={answers.onAcceptCertificate}
          onForget={answers.onForgetCertificate}
        />
      );
    }
  }
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
  answers,
}: {
  readonly item: NeedsYouItem;
  readonly readOnly: boolean;
  readonly onDismiss: (id: string) => void;
  readonly answers: Answers;
}): ReactNode {
  const since = formatInstant(item.since);
  return (
    <li className="flex flex-col gap-2 rounded-lg border p-4">
      <Content item={item} readOnly={readOnly} answers={answers} />
      <p className="text-muted-foreground text-sm">
        <Trans>
          Waiting since{" "}
          <time dateTime={item.since} translate="no">
            {since}
          </time>
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
        <output className="block">
          <Trans>
            You raised the Limit, and the run <code translate="no">{run}</code> resumes in the
            Daemon.
          </Trans>
        </output>
      );
    case "not_resumed":
      return (
        <div role="alert" className="text-destructive">
          <p>
            <Trans>
              You raised the Limit, but the engine did not resume the run{" "}
              <code translate="no">{run}</code>:
            </Trans>
          </p>
          <ErrorMessage error={raised.error} />
        </div>
      );
    case "waiting_for_script": {
      const command = raised.command;
      return (
        <div className="flex flex-col gap-2">
          <output className="block">
            <Trans>
              You raised the Limit. The run <code translate="no">{run}</code> waits for its script:
              start the program again with its Run ID to resume it.
            </Trans>
          </output>
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
    <output className="block">
      {copied === "copied" ? (
        <Trans>Copied the resume command.</Trans>
      ) : copied === "copy_refused" ? (
        <Trans>The browser refused to copy the resume command: select it and copy it.</Trans>
      ) : null}
    </output>
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
  onRemoveRefetchDomain,
  onDenyWallDomain,
  onAcceptCertificate,
  onForgetCertificate,
}: NeedsYouViewProps): ReactNode {
  const { t } = useLingui();
  const answers: Answers = {
    onRaise,
    onAllowConsentDomain,
    onRemoveRefetchDomain,
    onDenyWallDomain,
    onAcceptCertificate,
    onForgetCertificate,
  };
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
        <output className="block">
          <Trans>Reading what needs you…</Trans>
        </output>
      ) : state.kind === "failed" ? (
        <FailedList error={state.error} />
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
              answers={answers}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

/** The engine's refusal of the list. */
function FailedList({ error }: { readonly error: unknown }): ReactNode {
  return (
    <div role="alert">
      <p>
        <Trans>The engine could not list what needs you:</Trans>
      </p>
      <ErrorMessage error={error} />
    </div>
  );
}

/** What the engine refused, and its error. */
type Failure = {
  readonly kind:
    | "dismiss"
    | "raise"
    | "allow_consent_domain"
    | "remove_refetch_domain"
    | "deny_wall_domain"
    | "accept_certificate"
    | "forget_certificate";
  readonly error: unknown;
};

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
      setFailure({ kind: "dismiss", error });
    });
  };
  const raise = async (item: NeedsYouItem, stop: LimitStop, amount: number): Promise<void> => {
    try {
      await client.call("usage.raise", raiseParams(stop.limit, amount));
    } catch (error) {
      setFailure({ kind: "raise", error });
      return;
    }
    let answer: Raised;
    if (item.actions.includes("run.resume")) {
      try {
        await client.call("run.resume", { run_id: stop.run });
        answer = { kind: "resumed", run: stop.run };
      } catch (error) {
        answer = { kind: "not_resumed", run: stop.run, error };
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
      setFailure({ kind: "allow_consent_domain", error });
    });
  };
  /** Sends one answer, saying the engine's refusal as `kind`. */
  const answer = (kind: Failure["kind"], sent: Promise<unknown>): void => {
    setFailure(undefined);
    sent.catch((error: unknown) => {
      setFailure({ kind, error });
    });
  };
  const onRemoveRefetchDomain = (
    domain: string,
    insteadOfStar: readonly string[] | undefined,
  ): void => {
    answer(
      "remove_refetch_domain",
      client.call(
        "needs_you.remove_refetch_domain",
        insteadOfStar === undefined ? { domain } : { domain, instead_of_star: [...insteadOfStar] },
      ),
    );
  };
  const onDenyWallDomain = (domain: string, reason: string): void => {
    answer("deny_wall_domain", client.call("needs_you.deny_wall_domain", { domain, reason }));
  };
  const onAcceptCertificate = (host: string, key: string): void => {
    answer("accept_certificate", client.call("certs.accept", { host, spki_sha256: key }));
  };
  const onForgetCertificate = (host: string): void => {
    answer("forget_certificate", client.call("certs.forget", { host }));
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
        onRemoveRefetchDomain={onRemoveRefetchDomain}
        onDenyWallDomain={onDenyWallDomain}
        onAcceptCertificate={onAcceptCertificate}
        onForgetCertificate={onForgetCertificate}
      />
      {failure === undefined ? null : (
        <div role="alert" className="text-destructive">
          <p>
            <FailureText failure={failure} />
          </p>
          <ErrorMessage error={failure.error} />
        </div>
      )}
    </>
  );
}

/** What the engine refused, in words. */
function FailureText({ failure }: { readonly failure: Failure }): ReactNode {
  switch (failure.kind) {
    case "dismiss":
      return <Trans>The engine could not dismiss the item:</Trans>;
    case "raise":
      return <Trans>The engine did not raise the Limit:</Trans>;
    case "allow_consent_domain":
      return (
        <Trans>
          The engine could not add the domain to <code translate="no">consent.domains</code>:
        </Trans>
      );
    case "remove_refetch_domain":
      return (
        <Trans>
          The engine could not remove the domain from{" "}
          <code translate="no">wall.refetch_domains</code>:
        </Trans>
      );
    case "deny_wall_domain":
      return (
        <Trans>
          The engine could not add the domain to <code translate="no">wall.denied_domains</code>:
        </Trans>
      );
    case "accept_certificate":
      return <Trans>The engine did not accept the new key:</Trans>;
    case "forget_certificate":
      return <Trans>The engine did not forget the pin:</Trans>;
    default:
      return failure.kind satisfies never;
  }
}
