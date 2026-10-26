// The effective settings, as `config explain` shows them (ADR 0028): each
// key with its value, the layer that set it (a run value with its origin),
// the layer that locked it, an Operator policy's cap, whether it weakens a
// guarantee, and, for a key only the other Product honours, that Product's
// name, so a shared configuration file reads honestly (ADR 0087). Each row
// offers what the Console may edit of it (ADR 0031): a user-level value or
// a run's own value, never a project configuration's or a Recipe
// manifest's, which are files under version control, and never a run's
// value fixed for its life, which only a new run changes.
import { Trans, useLingui } from "@lingui/react/macro";
import { useId, useState, type ReactNode, type SubmitEvent } from "react";
import type { ExplainedSetting, Product } from "@goodfetch/collie-client/generated";
import { LAYERS, RUN_ORIGINS } from "#settings-layers.ts";
import { Form } from "#form.tsx";

/** Each Product's name, which no language translates (ADR 0090). */
const PRODUCTS: Readonly<Record<Product, string>> = {
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- a Product's name (ADR 0090).
  collie: "Collie",
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- a Product's name (ADR 0090).
  foxhound: "Foxhound",
};

/** A value a person gave a setting: a user-level value, or a run's own value. */
export interface SettingEdit {
  readonly layer: "user_configuration" | "run";
  readonly key: string;
  readonly value: unknown;
}

/** What the Console may edit of a setting. */
export type Editable =
  /** Its user-level value, in user configuration. */
  | { readonly kind: "user" }
  /** Its run's own value. */
  | { readonly kind: "run" }
  /** Nothing: a run's value fixed for its life, which only a new run changes. */
  | { readonly kind: "fixed_for_run" }
  /** Nothing: a value under version control, in the project configuration or a Recipe's manifest. */
  | { readonly kind: "versioned"; readonly layer: "project_configuration" | "recipe" }
  /** Nothing: a locked value, the other Product's, or a key no user-level value sets. */
  | { readonly kind: "none" };

/** What the Console may edit of `setting`, from the layer that set it, its lock and its marks. */
export function editable(setting: ExplainedSetting): Editable {
  if (!setting.applied) {
    return { kind: "none" };
  }
  switch (setting.layer) {
    case "run":
      return setting.fixed_for_run ? { kind: "fixed_for_run" } : { kind: "run" };
    case "project_configuration":
    case "recipe":
      return { kind: "versioned", layer: setting.layer };
    case "default":
    case "operator_policy":
    case "user_configuration":
      return setting.user_settable ? { kind: "user" } : { kind: "none" };
    default: {
      const exhaustive: never = setting.layer;
      return exhaustive;
    }
  }
}

/** A JSON value as `config explain` prints it. */
function json(value: unknown): string {
  return JSON.stringify(value) ?? "null";
}

/** The value `text` writes as JSON, or `undefined` when it is no JSON. */
function parsed(text: string): { readonly value: unknown } | undefined {
  try {
    return { value: JSON.parse(text) };
  } catch {
    return undefined;
  }
}

const smallButtonClassName =
  "rounded-md border px-2 py-1 text-sm font-medium focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50";

/** The edit of one setting's value in `layer`: a button, then a field with Save and Cancel. */
function ValueEditor({
  setting,
  layer,
  disabled,
  onEdit,
}: {
  readonly setting: ExplainedSetting;
  readonly layer: SettingEdit["layer"];
  readonly disabled: boolean;
  readonly onEdit: (edit: SettingEdit) => void;
}): ReactNode {
  const { t } = useLingui();
  const field = useId();
  const [open, setOpen] = useState(false);
  const [invalid, setInvalid] = useState(false);
  const key = setting.key;
  if (!open) {
    return (
      <button
        type="button"
        disabled={disabled}
        onClick={() => {
          setOpen(true);
        }}
        aria-label={layer === "run" ? t`Edit run value of ${key}` : t`Edit user value of ${key}`}
        className={smallButtonClassName}
      >
        {layer === "run" ? <Trans>Edit run value</Trans> : <Trans>Edit user value</Trans>}
      </button>
    );
  }
  const submit = (event: SubmitEvent<HTMLFormElement>): void => {
    event.preventDefault();
    const text = new FormData(event.currentTarget).get("value");
    const value = typeof text === "string" ? parsed(text) : undefined;
    if (value === undefined) {
      setInvalid(true);
      return;
    }
    setInvalid(false);
    setOpen(false);
    onEdit({ layer, key, value: value.value });
  };
  return (
    <Form onSubmit={submit} className="flex flex-col gap-1">
      <label htmlFor={field} className="text-sm">
        {layer === "run" ? (
          <Trans>
            The run's value of <span translate="no">{key}</span>, as JSON
          </Trans>
        ) : (
          <Trans>
            The user-level value of <span translate="no">{key}</span>, as JSON
          </Trans>
        )}
      </label>
      <input
        id={field}
        name="value"
        defaultValue={json(setting.value)}
        translate="no"
        className="rounded-md border bg-background px-2 py-1 font-mono text-sm"
      />
      {invalid ? (
        <p role="alert" className="text-destructive text-sm">
          <Trans>The value is no JSON.</Trans>
        </p>
      ) : null}
      <div className="flex gap-2">
        <button
          type="submit"
          disabled={disabled}
          className="rounded-md bg-primary px-2 py-1 text-sm font-medium text-primary-foreground focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:opacity-50"
        >
          <Trans>Save</Trans>
        </button>
        <button
          type="button"
          onClick={() => {
            setOpen(false);
            setInvalid(false);
          }}
          className={smallButtonClassName}
        >
          <Trans>Cancel</Trans>
        </button>
      </div>
    </Form>
  );
}

/** What a person may do with `setting`'s value here. */
function Edit({
  setting,
  disabled,
  onEdit,
}: {
  readonly setting: ExplainedSetting;
  readonly disabled: boolean;
  readonly onEdit: (edit: SettingEdit) => void;
}): ReactNode {
  const can = editable(setting);
  switch (can.kind) {
    case "user":
    case "run":
      return (
        <ValueEditor
          setting={setting}
          layer={can.kind === "run" ? "run" : "user_configuration"}
          disabled={disabled}
          onEdit={onEdit}
        />
      );
    case "fixed_for_run":
      return <Trans>Fixed for a run's life: start a new run to change it</Trans>;
    case "versioned":
      return can.layer === "recipe" ? (
        <Trans>Edit the Recipe's manifest, under version control</Trans>
      ) : (
        <Trans>Edit the project configuration file, under version control</Trans>
      );
    case "none":
      return null;
    default: {
      const exhaustive: never = can;
      return exhaustive;
    }
  }
}

/** The notes of `setting`, beside its layers: its cap, a weakening, and the Product that honours it when this engine does not. */
function Notes({ setting }: { readonly setting: ExplainedSetting }): ReactNode {
  const { i18n } = useLingui();
  const notes: ReactNode[] = [];
  if (setting.cap !== undefined && setting.cap !== null) {
    const cap = json(setting.cap);
    notes.push(
      <span key="cap" className="block">
        <Trans>
          Capped at <code translate="no">{cap}</code>
        </Trans>
      </span>,
    );
  }
  if (setting.weakens) {
    notes.push(
      <span key="weakens" className="block">
        <Trans>Weakens a guarantee</Trans>
      </span>,
    );
  }
  if (!setting.applied) {
    const products = new Intl.ListFormat(i18n.locale, { type: "conjunction" }).format(
      setting.products.map((product) => PRODUCTS[product]),
    );
    notes.push(
      <span key="product" className="block">
        <Trans>
          <span translate="no">{products}</span>'s setting, not applied by this engine
        </Trans>
      </span>,
    );
  }
  return notes;
}

/** Every effective setting, one row each, each with what a person may edit of it. */
export function EffectiveSettings({
  settings,
  readOnly = false,
  onEdit,
}: {
  readonly settings: readonly ExplainedSetting[];
  /** Whether the Console has no Connection now, which disables editing. */
  readonly readOnly?: boolean | undefined;
  /** Where a person's edit goes; without it, as on a Run report, the table has no Edit column. */
  readonly onEdit?: ((edit: SettingEdit) => void) | undefined;
}): ReactNode {
  const { i18n } = useLingui();
  return (
    <table className="w-full border-collapse text-start text-sm">
      <caption className="sr-only">
        <Trans>Effective settings</Trans>
      </caption>
      <thead className="border-b">
        <tr>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Setting</Trans>
          </th>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Value</Trans>
          </th>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Set by</Trans>
          </th>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Locked by</Trans>
          </th>
          <th scope="col" className="py-2 pe-4 font-medium">
            <Trans>Notes</Trans>
          </th>
          {onEdit === undefined ? null : (
            <th scope="col" className="py-2 font-medium">
              <Trans>Edit</Trans>
            </th>
          )}
        </tr>
      </thead>
      <tbody>
        {settings.map((setting) => (
          <tr key={setting.key} className="border-b align-top">
            <th scope="row" className="py-2 pe-4 font-mono font-normal" translate="no">
              {setting.key}
            </th>
            <td className="py-2 pe-4 font-mono break-all" translate="no">
              {json(setting.value)}
            </td>
            <td className="py-2 pe-4">
              {i18n._(
                setting.layer === "run" && setting.origin !== undefined && setting.origin !== null
                  ? RUN_ORIGINS[setting.origin]
                  : LAYERS[setting.layer],
              )}
            </td>
            <td className="py-2 pe-4">
              {setting.locked_by === undefined || setting.locked_by === null ? (
                <Trans>Not locked</Trans>
              ) : (
                i18n._(LAYERS[setting.locked_by])
              )}
            </td>
            <td className="py-2 pe-4">
              <Notes setting={setting} />
            </td>
            {onEdit === undefined ? null : (
              <td className="py-2">
                <Edit setting={setting} disabled={readOnly} onEdit={onEdit} />
              </td>
            )}
          </tr>
        ))}
      </tbody>
    </table>
  );
}
