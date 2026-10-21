// The Console's shell (ADR 0090): its first screen names the Product,
// "Goodfetch Collie", then calls it "Collie". Later screens fill it.
import { Trans } from "@lingui/react/macro";
import type { ReactNode } from "react";

/** The Console's shell. */
export function Shell(): ReactNode {
  return (
    <main className="mx-auto flex min-h-svh max-w-2xl flex-col justify-center gap-3 px-6">
      <h1 className="font-heading text-3xl font-semibold tracking-tight">
        <Trans>Goodfetch Collie</Trans>
      </h1>
      <p className="text-muted-foreground">
        <Trans>Collie's Console. The runs you start with Collie show here.</Trans>
      </p>
    </main>
  );
}
