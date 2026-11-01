// The names of the settings layers (ADR 0028), which the effective
// settings and an error's `limit` show, and of the run layer's origins
// (ADR 0083).
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import type { RunOrigin, SettingsLayer } from "@goodfetch/collie-client/generated";

/** Each layer's name. */
export const LAYERS: Readonly<Record<SettingsLayer, MessageDescriptor>> = {
  default: msg`Built-in default`,
  operator_policy: msg`Operator policy`,
  user_configuration: msg`User configuration`,
  project_configuration: msg`Project configuration`,
  recipe: msg`Recipe`,
  run: msg`Run`,
};

/** The run layer's name with each origin of its values (ADR 0083). */
export const RUN_ORIGINS: Readonly<Record<RunOrigin, MessageDescriptor>> = {
  argument: msg`Run, from a call's argument`,
  settings: msg`Run, from the run's settings`,
  flag: msg`Run, from a CLI flag`,
  form: msg`Run, from an Integration's form`,
};

/** Whether `name` is a settings layer's. */
export function isSettingsLayer(name: string): name is SettingsLayer {
  return Object.hasOwn(LAYERS, name);
}
