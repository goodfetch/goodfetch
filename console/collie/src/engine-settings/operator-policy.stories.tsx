// Each state of the Operator policy section (TESTING.md, Components):
// reading, a policy with an entry in each table, an empty one, a write
// made, a value that is no JSON, a `policy_conflict` with its offer to read
// the policy again, and the engine's refusal to read it.
import {
  PolicyConflictError,
  SettingInvalidError,
  type OperatorPolicyRead,
  type PolicyEntry,
} from "@goodfetch/collie-client/generated";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { OperatorPolicyView } from "./operator-policy.tsx";

/** A JSON value as the generated types write a setting's value. */
function json(value: unknown): PolicyEntry["value"] {
  // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a setting's value is any JSON value, which the generated type writes as an object.
  return value as PolicyEntry["value"];
}

const everyTable: OperatorPolicyRead = {
  path: "/home/ada/.goodfetch/1/operator-policy.toml",
  found: true,
  digest: "sha256:3f2a9c0d1e4b5a6f",
  entries: [
    { table: "settings", key: "timeout.navigation", value: json(45000) },
    { table: "lock", key: "network.resolve", value: json({ "staging.example": ["127.0.0.1"] }) },
    { table: "cap", key: "timeout.run", value: json(900000) },
  ],
};

const meta: Meta<typeof OperatorPolicyView> = {
  title: "Engine & settings/Operator policy",
  component: OperatorPolicyView,
  args: { onWrite: (): void => {}, onReadAgain: (): void => {} },
};

export default meta;

type Story = StoryObj<typeof OperatorPolicyView>;

/** Waiting for `config.policy_read`'s answer. */
export const Reading: Story = { args: { read: { kind: "reading" } } };

/** A policy that sets, locks and caps a setting each. */
export const EveryTable: Story = { args: { read: { kind: "read", policy: everyTable } } };

/** No policy file: it sets nothing. */
export const Empty: Story = {
  args: {
    read: {
      kind: "read",
      policy: { ...everyTable, found: false, digest: "sha256:e3b0c44298fc1c14", entries: [] },
    },
  },
};

/** A write made, with the version new runs name. */
export const Written: Story = {
  args: {
    read: { kind: "read", policy: everyTable },
    notice: {
      kind: "written",
      written: {
        digest: "sha256:9b1c0e7f2a3d4c5b",
        diff: [{ table: "cap", key: "timeout.run", before: json(900000), after: json(600000) }],
      },
    },
  },
};

/** The value given is no JSON, so nothing was sent. */
export const ValueNotJson: Story = {
  args: { read: { kind: "read", policy: everyTable }, notice: { kind: "invalid_value" } },
};

/** The file changed since it was read: nothing was written, and reading it again is offered. */
export const Conflict: Story = {
  args: {
    read: { kind: "read", policy: everyTable },
    notice: {
      kind: "refused",
      error: new PolicyConflictError(
        "the Operator policy changed since it was read: its digest is `sha256:9b1c0e7f2a3d4c5b`, not `sha256:3f2a9c0d1e4b5a6f`, so nothing was written",
        {
          fix: "read the Operator policy again with `config.policy_read`, then make the changes against what it holds now",
          details: { digest: "sha256:3f2a9c0d1e4b5a6f", current: "sha256:9b1c0e7f2a3d4c5b" },
        },
      ),
    },
  },
};

/** The engine could not read the policy. */
export const Failed: Story = {
  args: {
    read: {
      kind: "failed",
      error: new SettingInvalidError("`operator-policy.toml` does not parse", {
        details: { key: "operator-policy", reason: "expected a table" },
      }),
    },
  },
};
