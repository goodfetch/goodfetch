// "Needs you" (spec #591, stories 23, 25, 26 and 32; ADRs 0031, 0048, 0052
// and 0122): every item waiting for a person, the longest waiting first,
// each shown by its kind's renderer, a kind this build has none for shown by
// the engine's summary. Collie's storage kinds: a filesystem at the `low` or
// `critical` storage level with its folder, and a run stopped by storage,
// which a person resumes once space is freed or dismisses. A domain that
// showed a consent wall is offered for `consent.domains`, alone or as `*`,
// each offer beside the plain warning of what the person takes on and
// answered in one click, with no confirmation asked. Items leave the list as
// they are resolved, here, elsewhere or by their run.
import type { NeedsYouItem } from "@goodfetch/collie-client/generated";
import { Trans, useLingui } from "@lingui/react/macro";
import { useState, type ReactNode } from "react";
import { formatInstant } from "#format.ts";
import type { NeedsYouState } from "./needs-you-store.ts";

/** What "Needs you" shows. */
export interface NeedsYouViewProps {
  readonly state: NeedsYouState;
  /** Whether the Console has no Connection now, which disables every action. */
  readonly readOnly?: boolean | undefined;
  /** Dismisses the item `id`. */
  readonly onDismiss: (id: string) => void;
  /** Adds `domain`, or `*`, to `consent.domains` for the next runs. */
  readonly onAllowConsentDomain: (domain: string) => void;
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

/** A domain that showed a consent wall, with its two offers, each beside what it takes on. */
function ConsentWall({
  details,
  offered,
  readOnly,
  onAllow,
}: {
  readonly details: ConsentWallDetails;
  readonly offered: boolean;
  readonly readOnly: boolean;
  readonly onAllow: (domain: string) => void;
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
      {offered ? (
        <>
          <p className="text-sm">
            <Trans>
              Accepting the consent walls of <span translate="no">{domain}</span> gives consent in
              your name: the site may load its trackers and set their cookies, and under the GDPR
              that consent is yours. Every later run accepts them until you remove{" "}
              <span translate="no">{domain}</span> from <code translate="no">consent.domains</code>.
            </Trans>
          </p>
          <button
            type="button"
            disabled={readOnly}
            onClick={() => onAllow(domain)}
            className="self-start rounded-md border px-3 py-1.5 font-medium focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50"
          >
            <Trans>Accept {domain}'s consent walls in later runs</Trans>
          </button>
          <p className="text-sm">
            <Trans>
              Accepting consent walls on every domain gives consent in your name on every site that
              shows one: those sites may load their trackers and set their cookies, and under the
              GDPR that consent is yours. An Agent also no longer asks you before accepting a
              consent prompt on any domain. Every later run accepts them until you remove{" "}
              <code translate="no">*</code> from <code translate="no">consent.domains</code>.
            </Trans>
          </p>
          <button
            type="button"
            disabled={readOnly}
            onClick={() => onAllow("*")}
            className="self-start rounded-md border px-3 py-1.5 font-medium focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50"
          >
            <Trans>Accept consent walls on every domain</Trans>
          </button>
        </>
      ) : null}
    </>
  );
}

/** What an item says, by its kind's renderer, or by the engine's summary for a kind this build has none for. */
function Content({
  item,
  readOnly,
  onAllowConsentDomain,
}: {
  readonly item: NeedsYouItem;
  readonly readOnly: boolean;
  readonly onAllowConsentDomain: (domain: string) => void;
}): ReactNode {
  if (item.kind === "consent_wall") {
    const details = consentWallOf(item);
    if (details !== undefined) {
      return (
        <ConsentWall
          details={details}
          offered={item.actions.includes("needs_you.allow_consent_domain")}
          readOnly={readOnly}
          onAllow={onAllowConsentDomain}
        />
      );
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
  return <p translate="no">{item.summary}</p>;
}

/** One item, with since when it waits and the actions it offers. */
function Item({
  item,
  readOnly,
  onDismiss,
  onAllowConsentDomain,
}: {
  readonly item: NeedsYouItem;
  readonly readOnly: boolean;
  readonly onDismiss: (id: string) => void;
  readonly onAllowConsentDomain: (domain: string) => void;
}): ReactNode {
  const { i18n } = useLingui();
  const since = formatInstant(item.since, i18n.locale);
  return (
    <li className="flex flex-col gap-2 rounded-lg border p-4">
      <Content item={item} readOnly={readOnly} onAllowConsentDomain={onAllowConsentDomain} />
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
          className="self-start rounded-md border px-3 py-1.5 font-medium focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50"
        >
          <Trans>Dismiss</Trans>
        </button>
      ) : null}
    </li>
  );
}

/** "Needs you", as it shows `state`. */
export function NeedsYouView({
  state,
  readOnly = false,
  onDismiss,
  onAllowConsentDomain,
}: NeedsYouViewProps): ReactNode {
  const { t } = useLingui();
  return (
    <div className="flex flex-col gap-6">
      <h1 className="font-heading text-3xl font-semibold tracking-tight">
        <Trans>Needs you</Trans>
      </h1>
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

/** An answer the engine refused: which, and its message. */
interface Refusal {
  readonly answer: "dismiss" | "allow_consent_domain";
  readonly message: string;
}

/** "Needs you" over the shell's list, dismissing through `dismiss` and answering a consent wall through `allowConsentDomain`. */
export function NeedsYou({
  state,
  readOnly,
  dismiss,
  allowConsentDomain,
}: {
  readonly state: NeedsYouState;
  readonly readOnly: boolean;
  readonly dismiss: (id: string) => Promise<boolean>;
  readonly allowConsentDomain: (domain: string) => Promise<boolean>;
}): ReactNode {
  const [refusal, setRefusal] = useState<Refusal | undefined>(undefined);
  const refused =
    (answer: Refusal["answer"]) =>
    (error: unknown): void => {
      setRefusal({ answer, message: error instanceof Error ? error.message : String(error) });
    };
  const onDismiss = (id: string): void => {
    setRefusal(undefined);
    dismiss(id).catch(refused("dismiss"));
  };
  const onAllowConsentDomain = (domain: string): void => {
    setRefusal(undefined);
    allowConsentDomain(domain).catch(refused("allow_consent_domain"));
  };
  return (
    <>
      <NeedsYouView
        state={state}
        readOnly={readOnly}
        onDismiss={onDismiss}
        onAllowConsentDomain={onAllowConsentDomain}
      />
      {refusal === undefined ? null : (
        <p role="alert" className="text-destructive">
          {refusal.answer === "dismiss" ? (
            <Trans>
              The engine could not dismiss the item: <span translate="no">{refusal.message}</span>
            </Trans>
          ) : (
            <Trans>
              The engine could not add the domain to <code translate="no">consent.domains</code>:{" "}
              <span translate="no">{refusal.message}</span>
            </Trans>
          )}
        </p>
      )}
    </>
  );
}
