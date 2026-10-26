// Each state of Engine & settings (TESTING.md, Components): reading, the
// effective settings across every layer with a lock, a cap, a weakening and
// a key only Foxhound honours, the engine's refusal, a project
// configuration that is no TOML, and each row's edit.
import {
  SettingInvalidError,
  type ExplainedSetting,
  type SettingsExplanation,
} from "@goodfetch/collie-client/generated";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { EngineSettingsView } from "./engine-settings.tsx";

/** A setting `config.explain` answers, at the built-in default unless overridden. */
function setting(
  key: string,
  value: unknown,
  {
    cap,
    ...overrides
  }: Partial<Omit<ExplainedSetting, "key" | "value" | "cap">> & {
    readonly cap?: unknown;
  } = {},
): ExplainedSetting {
  return {
    key,
    products: ["collie", "foxhound"],
    applied: true,
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a setting's value is any JSON value, which the generated type writes as an object.
    value: value as ExplainedSetting["value"],
    layer: "default",
    weakens: false,
    ...overrides,
    // oxlint-disable-next-line typescript/no-unsafe-type-assertion -- a cap is any JSON value too.
    ...(cap === undefined ? {} : { cap: cap as NonNullable<ExplainedSetting["cap"]> }),
  };
}

const everyLayer: SettingsExplanation = {
  policy_version: "sha256:3f2a9c0d1e4b5a6f",
  files: [],
  settings: [
    setting("crawl.retry.max", 5, {
      products: ["foxhound"],
      applied: false,
      layer: "user_configuration",
    }),
    setting(
      "network.resolve",
      { "staging.example": ["127.0.0.1"] },
      { layer: "operator_policy", locked_by: "operator_policy" },
    ),
    setting("timeout.action", 30000),
    setting("timeout.first_byte", 10000, { layer: "project_configuration" }),
    setting("timeout.navigation", 45000, { layer: "user_configuration" }),
    setting("timeout.run", 600000, { layer: "run", origin: "flag", cap: 900000 }),
    setting("tls.accept_invalid_certs", ["staging.example"], {
      layer: "user_configuration",
      weakens: true,
    }),
  ],
};

const meta: Meta<typeof EngineSettingsView> = {
  title: "Engine & settings",
  component: EngineSettingsView,
  args: { onExplain: (): void => {} },
};

export default meta;

type Story = StoryObj<typeof EngineSettingsView>;

/** Waiting for `config.explain`'s answer. */
export const Reading: Story = { args: { explained: { kind: "reading" } } };

/** Every effective value with its layer and lock. */
export const EveryLayer: Story = {
  args: { explained: { kind: "explained", explanation: everyLayer } },
};

/** The engine refused to explain. */
export const Failed: Story = {
  args: {
    explained: {
      kind: "failed",
      error: new SettingInvalidError("`timeout.navigation` in configuration.toml is no duration", {
        fix: "set `timeout.navigation` to a number of milliseconds",
        details: { operation: "config.explain", key: "timeout.navigation" },
      }),
    },
  },
};

/** The project configuration picked is no TOML, so nothing was sent. */
export const ProjectNotToml: Story = {
  args: {
    explained: { kind: "explained", explanation: everyLayer },
    refusal: {
      kind: "project",
      file: "goodfetch.toml",
      reason: "Invalid TOML document: unterminated table key",
    },
  },
};

/** The Console has no Connection: the values shown stay, and explaining is disabled. */
export const ReadOnly: Story = {
  args: { explained: { kind: "explained", explanation: everyLayer }, readOnly: true },
};

/** Each row with what a person may edit of it: a user-level value, a run's own value, none of a project configuration's or a Recipe's, and a run's value fixed for its life. */
export const Editable: Story = {
  args: {
    explained: {
      kind: "explained",
      explanation: {
        files: [],
        settings: [
          setting("crawl.scope.strategy", "same_domain", {
            products: ["foxhound"],
            layer: "run",
            origin: "settings",
            fixed_for_run: true,
          }),
          setting("timeout.action", 30000, { user_settable: true }),
          setting("timeout.first_byte", 10000, { layer: "project_configuration" }),
          setting("timeout.navigation", 45000, { layer: "recipe" }),
          setting("timeout.run", 600000, { layer: "run", origin: "settings" }),
        ],
      },
    },
    runValues: "timeout.run = 600000",
    onEdit: (): void => {},
  },
};

/** A user-level value written, for new runs. */
export const UserValueWritten: Story = {
  args: {
    ...Editable.args,
    editNotice: { kind: "user_written", key: "timeout.action" },
  },
};
