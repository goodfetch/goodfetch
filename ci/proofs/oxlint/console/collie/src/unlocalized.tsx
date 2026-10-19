// A Console component that hard-codes its text, which the Console's lint
// configuration refuses (ADR 0073): ci/proofs/run.sh expects one
// `lingui(no-unlocalized-strings)` error.
export function Greeting(): React.JSX.Element {
  return <p>Your runs are ready</p>;
}
