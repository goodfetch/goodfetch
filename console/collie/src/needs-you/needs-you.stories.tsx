// Each state of "Needs you" (TESTING.md, Components): reading the list,
// nothing waiting, a filesystem at the low storage level, one at the
// critical level beside the run it stopped, a domain that showed a consent
// wall, with and without a Connection, a kind this build has no renderer
// for, the Console read-only, and the engine's refusal.
import type { NeedsYouItem } from "@goodfetch/collie-client/generated";
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

const wall: NeedsYouItem = {
  id: "consent_wall:walled.example",
  kind: "consent_wall",
  since: "2026-10-24T09:10:00Z",
  run: "nightly",
  summary: "walled.example showed a consent wall.",
  actions: ["needs_you.allow_consent_domain", "needs_you.dismiss"],
  details: { domain: "walled.example" },
};

const meta: Meta<typeof NeedsYouView> = {
  title: "Needs you",
  component: NeedsYouView,
  args: { onDismiss: (): void => {}, onAllowConsentDomain: (): void => {} },
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

/** A domain that showed a consent wall, offered for `consent.domains` alone or as `*`, each beside its warning. */
export const ConsentWall: Story = { args: { state: { kind: "listed", items: [wall] } } };

/** The same with no Connection: both offers are disabled. */
export const ConsentWallReadOnly: Story = {
  args: { state: { kind: "listed", items: [wall] }, readOnly: true },
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

/** The engine refused the list. */
export const Failed: Story = {
  args: { state: { kind: "failed", message: "the Run index cannot be read" } },
};
