// The effective settings, as `config explain` shows them (ADR 0028): each
// key with its value, the layer that set it (a run value with its origin),
// the layer that locked it, an Operator policy's cap, whether it weakens a
// guarantee, and, for a key only the other Product honours, that Product's
// name, so a shared configuration file reads honestly (ADR 0087).
import { Trans, useLingui } from "@lingui/react/macro";
import type { ReactNode } from "react";
import type { ExplainedSetting, Product } from "@goodfetch/collie-client/generated";
import { LAYERS, RUN_ORIGINS } from "#settings-layers.ts";

/** Each Product's name, which no language translates (ADR 0090). */
const PRODUCTS: Readonly<Record<Product, string>> = {
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- a Product's name (ADR 0090).
  collie: "Collie",
  // oxlint-disable-next-line lingui/no-unlocalized-strings -- a Product's name (ADR 0090).
  foxhound: "Foxhound",
};

/** A JSON value as `config explain` prints it. */
function json(value: unknown): string {
  return JSON.stringify(value) ?? "null";
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

/** Every effective setting, one row each. */
export function EffectiveSettings({
  settings,
}: {
  readonly settings: readonly ExplainedSetting[];
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
          <th scope="col" className="py-2 font-medium">
            <Trans>Notes</Trans>
          </th>
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
            <td className="py-2">
              <Notes setting={setting} />
            </td>
          </tr>
        ))}
      </tbody>
    </table>
  );
}
