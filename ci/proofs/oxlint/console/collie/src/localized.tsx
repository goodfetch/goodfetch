// The same component with its text from the Lingui catalog, which the
// Console's lint configuration accepts (ADR 0073).
import { Trans } from "@lingui/react/macro";

export function Greeting(): React.JSX.Element {
  return (
    <p>
      <Trans>Your runs are ready</Trans>
    </p>
  );
}
