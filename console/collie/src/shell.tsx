// The Console's shell (ADR 0090): its side menu, with a link to each
// screen of Collie's build, and the screen of the route the page opened
// at. Its first screen, at `/`, names the Product, "Goodfetch Collie", then
// calls it "Collie", and says which engine its Client reached, by Product
// and version, or that the page is not signed in. Each screen's route is
// plain data (`routes.ts`, ADR 0072), and the page's title names it.
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import { Trans, useLingui } from "@lingui/react/macro";
import { useEffect, useSyncExternalStore, type ReactNode } from "react";
import type { ConsoleClient } from "#client/client.ts";
import { EngineSettings } from "#engine-settings/engine-settings.tsx";
import { ENGINE_SETTINGS } from "#routes.ts";

/** What the shell shows the Console's link to its engine with. */
export interface ShellProps {
  /** The Console's Client; none when the page is not signed in. */
  readonly client: ConsoleClient | undefined;
  /** The route the page opened at, its address's path. */
  readonly route: string;
}

/** A screen of Collie's build: its route, its name, and what it shows over a Client. */
interface Screen {
  readonly route: string;
  readonly name: MessageDescriptor;
  readonly content: (client: ConsoleClient) => ReactNode;
}

/** Collie's screens, in the side menu's order. */
const SCREENS: readonly Screen[] = [
  {
    route: ENGINE_SETTINGS,
    name: msg`Engine & settings`,
    content: (client) => <EngineSettings client={client} />,
  },
];

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

/** The message of a page that is not signed in. */
function NotSignedIn(): ReactNode {
  return <Trans>Not signed in: run goodfetch-collie console to open the Console signed in.</Trans>;
}

/** The first screen. */
function Overview({ client }: { readonly client: ConsoleClient | undefined }): ReactNode {
  return (
    <>
      <h1 className="font-heading text-3xl font-semibold tracking-tight">
        <Trans>Goodfetch Collie</Trans>
      </h1>
      <p className="text-muted-foreground">
        <Trans>Collie's Console. The runs you start with Collie show here.</Trans>
      </p>
      <p role="status">{client === undefined ? <NotSignedIn /> : <Engine client={client} />}</p>
    </>
  );
}

/** A link of the side menu to `route`, marked when it is the page's. */
function MenuLink({
  route,
  current,
  children,
}: {
  readonly route: string;
  readonly current: boolean;
  readonly children: ReactNode;
}): ReactNode {
  return (
    <a
      href={route}
      aria-current={current ? "page" : undefined}
      className="rounded-sm underline-offset-4 hover:underline focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none aria-[current=page]:font-semibold"
    >
      {children}
    </a>
  );
}

/** The Console's shell. */
export function Shell({ client, route }: ShellProps): ReactNode {
  const { i18n, t } = useLingui();
  const screen = SCREENS.find((candidate) => candidate.route === route);
  const name = screen === undefined ? undefined : i18n._(screen.name);
  const title = name === undefined ? t`Goodfetch Collie` : t`${name} · Goodfetch Collie`;
  useEffect(() => {
    document.title = title;
  }, [title]);
  return (
    <div className="min-h-svh">
      <header className="border-b">
        <nav aria-label={t`Console`} className="mx-auto flex max-w-5xl gap-4 px-6 py-3">
          <MenuLink route="/" current={screen === undefined}>
            <Trans>Overview</Trans>
          </MenuLink>
          {SCREENS.map((candidate) => (
            <MenuLink key={candidate.route} route={candidate.route} current={candidate === screen}>
              {i18n._(candidate.name)}
            </MenuLink>
          ))}
        </nav>
      </header>
      {screen === undefined ? (
        <main className="mx-auto flex max-w-2xl flex-col justify-center gap-3 px-6 py-16">
          <Overview client={client} />
        </main>
      ) : (
        <main className="mx-auto flex max-w-5xl flex-col gap-3 px-6 py-8">
          {client === undefined ? (
            <p role="status">
              <NotSignedIn />
            </p>
          ) : (
            screen.content(client)
          )}
        </main>
      )}
    </div>
  );
}
