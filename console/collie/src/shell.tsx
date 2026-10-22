// The Console's shell (ADR 0090): its first screen names the Product,
// "Goodfetch Collie", then calls it "Collie", and says which engine its
// Client reached, by Product and version, or that the page is not signed
// in. Later screens fill it.
import { Trans } from "@lingui/react/macro";
import { useSyncExternalStore, type ReactNode } from "react";
import type { ConsoleClient } from "#client/client.ts";

/** What the shell shows the Console's link to its engine with. */
export interface ShellProps {
  /** The Console's Client; none when the page is not signed in. */
  readonly client: ConsoleClient | undefined;
}

/** Where the Client stands, and the engine it reached. */
function Engine({ client }: { readonly client: ConsoleClient }): ReactNode {
  const state = useSyncExternalStore(
    (changed) => client.watch(changed),
    () => client.state,
  );
  const engine = client.engine;
  if (state !== "connected" || engine === undefined) {
    return <Trans>Connecting to the engine…</Trans>;
  }
  const version = engine.productVersion;
  return engine.product === "foxhound" ? (
    <Trans>Connected to Foxhound {version}.</Trans>
  ) : (
    <Trans>Connected to Collie {version}.</Trans>
  );
}

/** The Console's shell. */
export function Shell({ client }: ShellProps): ReactNode {
  return (
    <main className="mx-auto flex min-h-svh max-w-2xl flex-col justify-center gap-3 px-6">
      <h1 className="font-heading text-3xl font-semibold tracking-tight">
        <Trans>Goodfetch Collie</Trans>
      </h1>
      <p className="text-muted-foreground">
        <Trans>Collie's Console. The runs you start with Collie show here.</Trans>
      </p>
      <p role="status">
        {client === undefined ? (
          <Trans>Not signed in: run goodfetch-collie console to open the Console signed in.</Trans>
        ) : (
          <Engine client={client} />
        )}
      </p>
    </main>
  );
}
