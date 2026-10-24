// "Needs you" (spec #591, stories 23, 25 and 32; ADRs 0031 and 0122): every
// item waiting for a person, the longest waiting first, each shown by its
// kind's renderer, a kind this build has none for shown by the engine's
// summary. Collie's storage kinds: a filesystem at the `low` or `critical`
// storage level with its folder, and a run stopped by storage, which a
// person resumes once space is freed or dismisses. Items leave the list as
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

/** What an item says, by its kind's renderer, or by the engine's summary for a kind this build has none for. */
function Content({ item }: { readonly item: NeedsYouItem }): ReactNode {
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
}: {
  readonly item: NeedsYouItem;
  readonly readOnly: boolean;
  readonly onDismiss: (id: string) => void;
}): ReactNode {
  const { i18n } = useLingui();
  const since = formatInstant(item.since, i18n.locale);
  return (
    <li className="flex flex-col gap-2 rounded-lg border p-4">
      <Content item={item} />
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
export function NeedsYouView({ state, readOnly = false, onDismiss }: NeedsYouViewProps): ReactNode {
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
            <Item key={item.id} item={item} readOnly={readOnly} onDismiss={onDismiss} />
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

/** "Needs you" over the shell's list, dismissing through `dismiss`. */
export function NeedsYou({
  state,
  readOnly,
  dismiss,
}: {
  readonly state: NeedsYouState;
  readonly readOnly: boolean;
  readonly dismiss: (id: string) => Promise<boolean>;
}): ReactNode {
  const [failure, setFailure] = useState<string | undefined>(undefined);
  const onDismiss = (id: string): void => {
    setFailure(undefined);
    dismiss(id).catch((error: unknown) => {
      setFailure(error instanceof Error ? error.message : String(error));
    });
  };
  return (
    <>
      <NeedsYouView state={state} readOnly={readOnly} onDismiss={onDismiss} />
      {failure === undefined ? null : (
        <p role="alert" className="text-destructive">
          <Trans>
            The engine could not dismiss the item: <span translate="no">{failure}</span>
          </Trans>
        </p>
      )}
    </>
  );
}
