// The effective settings, as `config explain` shows them (ADR 0028): each
// key with its value, the layer that set it (a run value with its origin),
// the layer that locked it, an Operator policy's cap, whether it weakens a
// guarantee, and, for a key only the other Product honours, that Product's
// name, so a shared configuration file reads honestly (ADR 0087). Each row
// offers what the Console may edit of it (ADR 0031): a user-level value or
// a run's own value, never a project configuration's or a Recipe
// manifest's, which are files under version control, and never a run's
// value fixed for its life, which only a new run changes.
//
// The table is a grid (WAI-ARIA's data grid pattern, ADR 0073): it takes one
// Tab stop however many settings there are, and the arrow keys, Home, End,
// Page Up and Page Down move between its cells, onto a cell's Edit button
// where it has one.
import { Trans, useLingui } from "@lingui/react/macro";
import {
  useId,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type ReactNode,
  type SubmitEvent,
} from "react";
import type { ExplainedSetting, Product } from "@goodfetch/collie-client/generated";
import { LAYERS, RUN_ORIGINS } from "#settings-layers.ts";
import { Form } from "#form.tsx";
import { ScrollRegion } from "#scroll-region.tsx";

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
  tabIndex,
  onEdit,
}: {
  readonly setting: ExplainedSetting;
  readonly layer: SettingEdit["layer"];
  readonly disabled: boolean;
  /** The button's place in the grid's one Tab stop. */
  readonly tabIndex: number;
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
        tabIndex={tabIndex}
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
  tabIndex,
  onEdit,
}: {
  readonly setting: ExplainedSetting;
  readonly disabled: boolean;
  /** Its button's place in the grid's one Tab stop, when it has one. */
  readonly tabIndex: number;
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
          tabIndex={tabIndex}
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

/** A cell of the grid, its row counted from the header's. */
interface Cell {
  readonly row: number;
  readonly column: number;
}

/** How many rows Page Up and Page Down move. */
const PAGE_ROWS = 10;

/** The cell `key` moves focus to from `from` in a grid of `rows` by `columns`, or `undefined` for a key the grid leaves alone. */
function moved(
  key: string,
  toCorner: boolean,
  from: Cell,
  { rows, columns }: { readonly rows: number; readonly columns: number },
): Cell | undefined {
  const at = (row: number, column: number): Cell => ({
    row: Math.min(Math.max(row, 0), rows - 1),
    column: Math.min(Math.max(column, 0), columns - 1),
  });
  switch (key) {
    case "ArrowUp":
      return at(from.row - 1, from.column);
    case "ArrowDown":
      return at(from.row + 1, from.column);
    case "ArrowLeft":
      return at(from.row, from.column - 1);
    case "ArrowRight":
      return at(from.row, from.column + 1);
    case "PageUp":
      return at(from.row - PAGE_ROWS, from.column);
    case "PageDown":
      return at(from.row + PAGE_ROWS, from.column);
    case "Home":
      return toCorner ? at(0, 0) : at(from.row, 0);
    case "End":
      return toCorner ? at(rows - 1, columns - 1) : at(from.row, columns - 1);
    default:
      return undefined;
  }
}

/** The grid's cell that holds `target`, or `undefined` outside a cell or inside an open edit, whose field keeps its keys. */
function cellOf(target: EventTarget): Cell | undefined {
  if (!(target instanceof Element) || target.closest("form") !== null) {
    return undefined;
  }
  const cell = target.closest("th,td");
  const row = cell?.parentElement;
  return cell instanceof HTMLTableCellElement && row instanceof HTMLTableRowElement
    ? { row: row.rowIndex, column: cell.cellIndex }
    : undefined;
}

/** Focuses `cell` of `table`: its widget when it has one, else the cell itself. */
function focusCell(table: HTMLTableElement, { row, column }: Cell): void {
  const element = table.rows[row]?.cells[column];
  if (element !== undefined) {
    (element.querySelector<HTMLElement>("button:enabled,input") ?? element).focus();
  }
}

/** A cell's ring when it holds the grid's focus. */
const cellClassName =
  "focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring";

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
  const { i18n, t } = useLingui();
  const caption = useId();
  const rows = settings.length + 1;
  const columns = onEdit === undefined ? 5 : 6;
  // The cell holding the grid's one Tab stop: the last one focused, kept inside the grid as settings go.
  const [focused, setFocused] = useState<Cell>({ row: 0, column: 0 });
  const stop: Cell = {
    row: Math.min(focused.row, rows - 1),
    column: Math.min(focused.column, columns - 1),
  };
  /** The Tab index of the cell at `row` and `column`, or of its widget: 0 for the grid's stop. */
  const tabIndexOf = (row: number, column: number): number =>
    row === stop.row && column === stop.column ? 0 : -1;
  const follow = (event: FocusEvent<HTMLTableElement>): void => {
    const cell = cellOf(event.target);
    if (cell !== undefined && (cell.row !== focused.row || cell.column !== focused.column)) {
      setFocused(cell);
    }
  };
  const move = (event: KeyboardEvent<HTMLTableElement>): void => {
    const from = cellOf(event.target);
    const to =
      from === undefined
        ? undefined
        : moved(event.key, event.ctrlKey || event.metaKey, from, { rows, columns });
    if (to !== undefined) {
      event.preventDefault();
      focusCell(event.currentTarget, to);
    }
  };
  return (
    <ScrollRegion label={t`Effective settings, scrollable`} focusable={false}>
      <table
        role="grid"
        aria-labelledby={caption}
        onKeyDown={move}
        onFocus={follow}
        className="w-full border-collapse text-start text-sm"
      >
        <caption id={caption} className="sr-only">
          <Trans>Effective settings</Trans>
        </caption>
        <thead className="border-b">
          <tr>
            <th
              scope="col"
              tabIndex={tabIndexOf(0, 0)}
              className={`py-2 pe-4 font-medium ${cellClassName}`}
            >
              <Trans>Setting</Trans>
            </th>
            <th
              scope="col"
              tabIndex={tabIndexOf(0, 1)}
              className={`py-2 pe-4 font-medium ${cellClassName}`}
            >
              <Trans>Value</Trans>
            </th>
            <th
              scope="col"
              tabIndex={tabIndexOf(0, 2)}
              className={`py-2 pe-4 font-medium ${cellClassName}`}
            >
              <Trans>Set by</Trans>
            </th>
            <th
              scope="col"
              tabIndex={tabIndexOf(0, 3)}
              className={`py-2 pe-4 font-medium ${cellClassName}`}
            >
              <Trans>Locked by</Trans>
            </th>
            <th
              scope="col"
              tabIndex={tabIndexOf(0, 4)}
              className={`py-2 pe-4 font-medium ${cellClassName}`}
            >
              <Trans>Notes</Trans>
            </th>
            {onEdit === undefined ? null : (
              <th
                scope="col"
                tabIndex={tabIndexOf(0, 5)}
                className={`py-2 font-medium ${cellClassName}`}
              >
                <Trans>Edit</Trans>
              </th>
            )}
          </tr>
        </thead>
        <tbody>
          {settings.map((setting, index) => {
            const row = index + 1;
            const kind = editable(setting).kind;
            return (
              <tr key={setting.key} className="border-b align-top">
                <th
                  scope="row"
                  tabIndex={tabIndexOf(row, 0)}
                  className={`py-2 pe-4 font-mono font-normal ${cellClassName}`}
                  translate="no"
                >
                  {setting.key}
                </th>
                <td
                  tabIndex={tabIndexOf(row, 1)}
                  className={`py-2 pe-4 font-mono break-all ${cellClassName}`}
                  translate="no"
                >
                  {json(setting.value)}
                </td>
                <td tabIndex={tabIndexOf(row, 2)} className={`py-2 pe-4 ${cellClassName}`}>
                  {i18n._(
                    setting.layer === "run" &&
                      setting.origin !== undefined &&
                      setting.origin !== null
                      ? RUN_ORIGINS[setting.origin]
                      : LAYERS[setting.layer],
                  )}
                </td>
                <td tabIndex={tabIndexOf(row, 3)} className={`py-2 pe-4 ${cellClassName}`}>
                  {setting.locked_by === undefined || setting.locked_by === null ? (
                    <Trans>Not locked</Trans>
                  ) : (
                    i18n._(LAYERS[setting.locked_by])
                  )}
                </td>
                <td tabIndex={tabIndexOf(row, 4)} className={`py-2 pe-4 ${cellClassName}`}>
                  <Notes setting={setting} />
                </td>
                {onEdit === undefined ? null : (
                  <td
                    // A cell with an enabled Edit button gives the grid's stop to the button.
                    tabIndex={
                      (kind === "user" || kind === "run") && !readOnly
                        ? undefined
                        : tabIndexOf(row, 5)
                    }
                    className={`py-2 ${cellClassName}`}
                  >
                    <Edit
                      setting={setting}
                      disabled={readOnly}
                      tabIndex={tabIndexOf(row, 5)}
                      onEdit={onEdit}
                    />
                  </td>
                )}
              </tr>
            );
          })}
        </tbody>
      </table>
    </ScrollRegion>
  );
}
