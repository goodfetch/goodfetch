// Each state of "Needs you" (TESTING.md, Components): reading the list,
// nothing waiting, a filesystem at the low storage level, one at the
// critical level beside the run it stopped, a kind this build has no
// renderer for, the Console read-only, and the engine's refusal; runs
// stopped by a Limit, an SDK script's and a Recipe's priced one, and what a
// person's raise left: a run waiting for its script, with its resume
// command, and one resumed in the Daemon; a domain that showed a consent
// wall, with and without a Connection; stops of the refetch without the
// site's stored data, one per layer that lists the domain; and a
// certificate whose key changed.
import { RunStateUnavailableError, type NeedsYouItem } from "@goodfetch/collie-client/generated";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { NeedsYouView } from "./needs-you.tsx";

const FOLDER = "/home/ada/.goodfetch";

const low: NeedsYouItem = {
  id: `storage_level:${FOLDER}`,
  kind: "storage_level",
  since: "2026-10-24T09:00:00Z",
  summary: `The filesystem holding ${FOLDER} is at the low storage level.`,
  actions: [],
  details: { level: "low", folder: FOLDER },
};

const critical: NeedsYouItem = {
  ...low,
  summary: `The filesystem holding ${FOLDER} is at the critical storage level.`,
  details: { level: "critical", folder: FOLDER },
};

const stopped: NeedsYouItem = {
  id: "run_stopped_by_storage:nightly:2026-10-24T09:05:00Z",
  kind: "run_stopped_by_storage",
  since: "2026-10-24T09:05:00Z",
  run: "nightly",
  summary: "The Run nightly stopped: the filesystem holding its folder filled.",
  actions: ["needs_you.dismiss"],
  details: { folder: `${FOLDER}/1/runs/nightly` },
};

const byLimit: NeedsYouItem = {
  id: "run_stopped_by_limit:capped:2026-10-24T09:10:00Z",
  kind: "run_stopped_by_limit",
  since: "2026-10-24T09:10:00Z",
  run: "capped",
  summary: "The Run capped stopped: a Limit on requests has nothing left.",
  actions: ["usage.raise", "needs_you.dismiss"],
  details: {
    limit: {
      scope: { kind: "install", id: "" },
      usage_kind: "requests",
      period: { every: "month", time_zone: "Europe/Paris" },
      resets_at: "2026-10-31T23:00:00Z",
    },
    amount: 1000,
    needs: "collie",
  },
};

const recipeByPrice: NeedsYouItem = {
  id: "run_stopped_by_limit:nightly-prices:2026-10-24T09:20:00Z",
  kind: "run_stopped_by_limit",
  since: "2026-10-24T09:20:00Z",
  run: "nightly-prices",
  summary:
    "The Run nightly-prices of the Recipe shop-prices stopped: a Limit on proxy_bytes has nothing left.",
  actions: ["usage.raise", "run.resume", "needs_you.dismiss"],
  details: {
    limit: {
      scope: { kind: "daemon", id: "1" },
      usage_kind: "proxy_bytes",
      currency: "USD",
    },
    amount: 5_000_000,
    needs: "collie",
    recipe: "shop-prices",
  },
};

const wall: NeedsYouItem = {
  id: "consent_wall:walled.example",
  kind: "consent_wall",
  since: "2026-10-24T09:10:00Z",
  run: "nightly",
  summary: "walled.example showed a consent wall.",
  actions: ["needs_you.allow_consent_domain", "needs_you.dismiss"],
  details: { domain: "walled.example" },
};

/** A stop of the refetch without the site's stored data on `domain`, listed in `layer`'s `file`, or only by `*` there. */
function refetchStopped(
  domain: string,
  layer: "user_configuration" | "operator_policy" | "project_configuration",
  file: string,
  onlyStar: boolean,
): NeedsYouItem {
  return {
    id: `wall_refetch_stopped:${domain}`,
    kind: "wall_refetch_stopped",
    since: "2026-10-24T09:30:00Z",
    run: "nightly",
    summary: `${domain} refused a page fetched again without its stored cookies and storage.`,
    actions: [
      ...(layer === "project_configuration" ? [] : ["needs_you.remove_refetch_domain"]),
      "needs_you.deny_wall_domain",
      "needs_you.dismiss",
    ],
    details: { domain, cause: "challenge", layer, file, only_star: onlyStar },
  };
}

const changedCertificate: NeedsYouItem = {
  id: "certificate_changed:intranet.example",
  kind: "certificate_changed",
  since: "2026-10-24T09:40:00Z",
  run: "nightly",
  summary: "intranet.example presented another key.",
  actions: ["certs.accept", "certs.forget"],
  details: {
    host: "intranet.example",
    person: "os_user",
    pinned_spki_sha256: "sha256/pinned-key-of-the-example-host",
    presented_spki_sha256: "sha256/changed-key-of-the-example-host",
  },
};

const meta: Meta<typeof NeedsYouView> = {
  title: "Needs you",
  component: NeedsYouView,
  args: {
    onDismiss: (): void => {},
    onRaise: (): void => {},
    onCopy: (): void => {},
    onAllowConsentDomain: (): void => {},
    onRemoveRefetchDomain: (): void => {},
    onDenyWallDomain: (): void => {},
    onAcceptCertificate: (): void => {},
    onForgetCertificate: (): void => {},
  },
};

export default meta;

type Story = StoryObj<typeof NeedsYouView>;

/** Waiting for `needs_you.list`'s answer. */
export const Reading: Story = { args: { state: { kind: "reading" } } };

/** Nothing waits for a person. */
export const Empty: Story = { args: { state: { kind: "listed", items: [] } } };

/** A filesystem the runs write to is at the low storage level. */
export const StorageLow: Story = { args: { state: { kind: "listed", items: [low] } } };

/** A filesystem at the critical level, and the run it stopped, which a person may dismiss. */
export const StorageCritical: Story = {
  args: { state: { kind: "listed", items: [critical, stopped] } },
};

/** A kind this build has no renderer for, such as a newer engine's: shown by its summary. */
export const UnknownKind: Story = {
  args: {
    state: {
      kind: "listed",
      items: [
        {
          id: "schedule_auto_paused:nightly",
          kind: "schedule_auto_paused",
          since: "2026-10-24T08:00:00Z",
          schedule: "nightly",
          summary: "The Schedule nightly paused itself after three failed occurrences.",
          actions: [],
        },
      ],
    },
  },
};

/** The Console has no Connection: the list stays, and dismissing is disabled. */
export const ReadOnly: Story = {
  args: { state: { kind: "listed", items: [critical, stopped] }, readOnly: true },
};

/** Runs stopped by a Limit: an SDK script's on the install's monthly Limit, and a Recipe's on a Daemon's price Limit with no period. */
export const StoppedByLimit: Story = {
  args: { state: { kind: "listed", items: [byLimit, recipeByPrice] } },
};

/** A person raised the Limits: the SDK script's run waits for its script, its resume command copied, and the Recipe's run resumes in the Daemon. */
export const LimitRaised: Story = {
  args: {
    state: { kind: "listed", items: [] },
    raised: [
      {
        kind: "waiting_for_script",
        run: "capped",
        command: "goodfetch-collie run resume capped",
      },
      { kind: "resumed", run: "nightly-prices" },
    ],
    copied: "copied",
  },
};

/** A domain that showed a consent wall, offered for `consent.domains` alone or as `*`, each beside its warning. */
export const ConsentWall: Story = { args: { state: { kind: "listed", items: [wall] } } };

/** The same with no Connection: both offers are disabled. */
export const ConsentWallReadOnly: Story = {
  args: { state: { kind: "listed", items: [wall] }, readOnly: true },
};

/** A refetch without the site's stored data stopped on a domain each layer lists: user configuration, `*` alone, and project configuration, each with its file. */
export const RefetchStopped: Story = {
  args: {
    state: {
      kind: "listed",
      items: [
        refetchStopped(
          "walled.example",
          "user_configuration",
          "/home/ada/.goodfetch/1/configuration.toml",
          false,
        ),
        refetchStopped(
          "starred.example",
          "operator_policy",
          "/home/ada/.goodfetch/1/operator-policy.toml",
          true,
        ),
        refetchStopped(
          "project.example",
          "project_configuration",
          "/home/ada/shop/goodfetch.toml",
          false,
        ),
      ],
    },
  },
};

/** A pinned host whose certificate's key changed, with both keys and its person's two answers. */
export const CertificateChanged: Story = {
  args: { state: { kind: "listed", items: [changedCertificate] } },
};

/** The engine refused the list. */
export const Failed: Story = {
  args: {
    state: {
      kind: "failed",
      error: new RunStateUnavailableError("the Run index cannot be read", {
        details: { operation: "needs_you.list", path: "/srv/goodfetch/1/run-index.sqlite" },
      }),
    },
  },
};
