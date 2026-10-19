// The same component with its text from the Lingui catalog, which the
// Console's lint configuration accepts (ADR 0073): a `<Trans>` element, and a
// message through `i18n._`, which the rule's callee list lets through (its
// matcher is picomatch, through the workspace's override of micromatch).
import { i18n } from "@lingui/core";
import { Trans } from "@lingui/react/macro";

const label = i18n._("Runs");

export function Greeting(): React.JSX.Element {
  return (
    <p aria-label={label}>
      <Trans>Your runs are ready</Trans>
    </p>
  );
}
