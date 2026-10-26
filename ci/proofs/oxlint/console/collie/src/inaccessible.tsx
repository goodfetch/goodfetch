// A Console component whose image has no text alternative and whose clickable
// `<div>` no keyboard reaches, which the Console's lint configuration refuses
// (ADR 0073: every `jsx-a11y` rule is an error): ci/proofs/run.sh expects
// `jsx-a11y(alt-text)`, `jsx-a11y(click-events-have-key-events)` and
// `jsx-a11y(no-static-element-interactions)`, and nothing on `localized.tsx`.
import { Trans } from "@lingui/react/macro";

export function Logo({ onOpen }: { readonly onOpen: () => void }): React.JSX.Element {
  return (
    <div onClick={onOpen}>
      <img src="/logo.svg" />
      <Trans>Open</Trans>
    </div>
  );
}
