// The Console's shell (ADR 0090): its side menu, with a link to each
// screen of Collie's build, "Needs you" counting what waits for a person
// as the list changes (ADR 0031), and the screen of the route the page
// opened at. Its first screen, at `/`, names the build's Product,
// "Goodfetch Collie" or "Goodfetch Foxhound", then calls it "Collie" or
// "Foxhound", and says which engine its Client reached, by Product
// and version, or that the page is not signed in, and offers a monthly
// Limit for the install while none holds (ADR 0041); the Runs screen lists
// the runs, and a run's page, at `/runs/<run-id>`, one of them. Each
// screen's route is plain data (`routes.ts`, ADR 0072), its `$` segments
// the screen's parameters, and the page's title names it; a screen that
// keeps its view in the address reads and replaces the address's query.
// While the Client has no Connection, after its first or once its first
// attempt failed, a banner says the page is read-only until it reconnects,
// which it does by itself, and each screen disables its actions (ADR
// 0031). A Product's build entry passes its build (`console-build.ts`):
// Foxhound's carries Foxhound's views of a run, which a run's page shows
// for a run that needs Foxhound (ADRs 0087 and 0098).
import type { Product } from "@goodfetch/collie-client/generated";
import type { MessageDescriptor } from "@lingui/core";
import { msg } from "@lingui/core/macro";
import { Plural, Trans, useLingui } from "@lingui/react/macro";
import { useEffect, useMemo, useSyncExternalStore, type ReactNode } from "react";
import { NO_ADDRESS, type Address } from "#address.ts";
import type { ConsoleClient } from "#client/client.ts";
import { COLLIE_BUILD, type ConsoleBuild, type RunView } from "#console-build.ts";
import { EngineSettings } from "#engine-settings/engine-settings.tsx";
import { formatCount } from "#format.ts";
import { NeedsYouStore, READING, type NeedsYouState } from "#needs-you/needs-you-store.ts";
import { NeedsYou } from "#needs-you/needs-you.tsx";
import type { OfflineCache } from "#offline/offline.ts";
import { ENGINE_SETTINGS, NEEDS_YOU, RUN, RUNS, paramsOf, type RouteParams } from "#routes.ts";
import { RunPage } from "#runs/run-page.tsx";
import { Runs } from "#runs/runs.tsx";
import { InstallLimitOffer } from "#usage/install-limit-offer.tsx";

/** What the shell shows the Console's link to its engine with. */
export interface ShellProps {
  /** The Console's Client; none when the page is not signed in. */
  readonly client: ConsoleClient | undefined;
  /** The route the page opened at, its address's path. */
  readonly route: string;
  /** The page's address, whose query a screen keeps its view in; none by default. */
  readonly address?: Address | undefined;
  /** The browser's offline cache of the page's sign-in; none while `console.offline_cache` is off. */
  readonly offline?: OfflineCache | undefined;
  /** Signs the page out, wiping its offline cache; none for a page that cannot. */
  readonly onSignOut?: (() => void) | undefined;
  /** The Product's build the page is; Collie's by default. */
  readonly build?: ConsoleBuild | undefined;
}

/** What a screen shows over: the Console's Client, the page's address, the shell's "Needs you" and whether the page is read-only. */
interface ScreenContext {
  readonly client: ConsoleClient;
  readonly offline: OfflineCache | undefined;
  readonly address: Address;
  readonly needsYou: ShellNeedsYou;
  readonly readOnly: boolean;
  /** The views of a run that needs Foxhound the build carries. */
  readonly runViews: readonly RunView[];
}

/** A screen of Collie's build: its route, its name, whether the side menu links it, and what it shows with its route's parameters. */
interface Screen {
  readonly route: string;
  readonly name: MessageDescriptor;
  readonly inMenu: boolean;
  readonly content: (context: ScreenContext, params: RouteParams) => ReactNode;
}

/** Collie's screens, the side menu's in its order. */
const SCREENS: readonly Screen[] = [
  {
    route: NEEDS_YOU,
    name: msg`Needs you`,
    inMenu: true,
    content: ({ client, needsYou, readOnly }) => (
      <NeedsYou
        client={client}
        state={needsYou.state}
        readOnly={readOnly}
        dismiss={needsYou.dismiss}
      />
    ),
  },
  {
    route: RUNS,
    name: msg`Runs`,
    inMenu: true,
    content: ({ client, address, offline }) => (
      <Runs client={client} address={address} offline={offline} />
    ),
  },
  {
    route: ENGINE_SETTINGS,
    name: msg`Engine & settings`,
    inMenu: true,
    content: ({ client }) => <EngineSettings client={client} />,
  },
  {
    route: RUN,
    name: msg`Run`,
    inMenu: false,
    content: ({ client, readOnly, offline, runViews }, { run = "" }) => (
      <RunPage
        key={run}
        client={client}
        runId={run}
        readOnly={readOnly}
        offline={offline}
        runViews={runViews}
      />
    ),
  },
];

/** The screen at `route`, a page's path, with the parameters its route takes from it. */
function screenAt(
  route: string,
): { readonly screen: Screen; readonly params: RouteParams } | undefined {
  for (const screen of SCREENS) {
    const params = paramsOf(screen.route, route);
    if (params !== undefined) {
      return { screen, params };
    }
  }
  return undefined;
}

/** "Needs you" as the shell holds it: the list now, and how to dismiss an item. */
interface ShellNeedsYou {
  readonly state: NeedsYouState;
  readonly dismiss: (id: string) => Promise<boolean>;
}

/** "Needs you" over `client`, kept current while the shell shows. */
function useNeedsYou(client: ConsoleClient | undefined): ShellNeedsYou {
  const store = useMemo(
    () => (client === undefined ? undefined : new NeedsYouStore(client)),
    [client],
  );
  useEffect(() => store?.start(), [store]);
  const state = useSyncExternalStore(
    (changed) => store?.watch(changed) ?? (() => {}),
    () => store?.snapshot() ?? READING,
  );
  return {
    state,
    dismiss: async (id) => (store === undefined ? false : store.dismiss(id)),
  };
}

/** How many items wait in `state`; none before the list's first answer. */
function countOf(state: NeedsYouState): number {
  return state.kind === "listed" ? state.items.length : 0;
}

/** Whether the page is read-only: its Client has no Connection now, after one it had or one it failed to open. */
function useReadOnly(client: ConsoleClient | undefined): boolean {
  const state = useSyncExternalStore(
    (changed) => client?.watch(changed) ?? (() => {}),
    () => client?.state,
  );
  if (client === undefined) {
    return false;
  }
  return state === "read_only" || (state === "reconnecting" && client.engine !== undefined);
}

/** Whether the Client has a Connection now. */
function useConnected(client: ConsoleClient | undefined): boolean {
  const state = useSyncExternalStore(
    (changed) => client?.watch(changed) ?? (() => {}),
    () => client?.state,
  );
  return state === "connected";
}

/** The banner of a page with no Connection. */
function ReadOnlyBanner(): ReactNode {
  return (
    <div role="alert" className="border-b bg-muted px-6 py-2 text-center font-medium">
      <Trans>
        No connection to the engine: this page is read-only until the Console reconnects, which it
        does by itself.
      </Trans>
    </div>
  );
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

/** The message of a page of `product`'s build that is not signed in. */
function NotSignedIn({ product }: { readonly product: Product }): ReactNode {
  return product === "foxhound" ? (
    <Trans>Not signed in: run goodfetch-foxhound console to open the Console signed in.</Trans>
  ) : (
    <Trans>Not signed in: run goodfetch-collie console to open the Console signed in.</Trans>
  );
}

/**
 * How many items wait in "Needs you", in a live region a screen reader
 * announces as the count changes (ADR 0073), out of sight beside the side
 * menu's count. Each form is an element of its own, keyed, so React replaces
 * the element, never a text node a page translator moved (react#11538).
 */
function NeedsYouAnnouncement({ state }: { readonly state: NeedsYouState }): ReactNode {
  const waiting = countOf(state);
  return (
    <output className="sr-only">
      {state.kind !== "listed" ? null : waiting === 0 ? (
        <span key="none">
          <Trans>Nothing needs you</Trans>
        </span>
      ) : (
        <span key="some">
          <Plural value={waiting} one="# item needs you" other="# items need you" />
        </span>
      )}
    </output>
  );
}

/** The first screen of `product`'s build. */
function Overview({
  client,
  product,
}: {
  readonly client: ConsoleClient | undefined;
  readonly product: Product;
}): ReactNode {
  return (
    <>
      <h1 className="font-heading text-3xl font-semibold tracking-tight">
        {product === "foxhound" ? (
          <Trans>Goodfetch Foxhound</Trans>
        ) : (
          <Trans>Goodfetch Collie</Trans>
        )}
      </h1>
      <p className="text-muted-foreground">
        {product === "foxhound" ? (
          <Trans>
            Foxhound's Console. The runs you start with Foxhound or Collie show under Runs.
          </Trans>
        ) : (
          <Trans>Collie's Console. The runs you start with Collie show under Runs.</Trans>
        )}
      </p>
      <output className="block">
        {client === undefined ? <NotSignedIn product={product} /> : <Engine client={client} />}
      </output>
      {client === undefined ? null : <InstallLimitOffer client={client} />}
    </>
  );
}

/** The page's title: the screen's name, if any, then the build's Product. */
function useTitle(product: Product, name: string | undefined): string {
  const { t } = useLingui();
  if (product === "foxhound") {
    return name === undefined ? t`Goodfetch Foxhound` : t`${name} · Goodfetch Foxhound`;
  }
  return name === undefined ? t`Goodfetch Collie` : t`${name} · Goodfetch Collie`;
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
export function Shell({
  client,
  route,
  address = NO_ADDRESS,
  offline,
  onSignOut,
  build = COLLIE_BUILD,
}: ShellProps): ReactNode {
  const { i18n, t } = useLingui();
  const at = screenAt(route);
  const screen = at?.screen;
  const name = screen === undefined ? undefined : i18n._(screen.name);
  const title = useTitle(build.product, name);
  useEffect(() => {
    document.title = title;
  }, [title]);
  const readOnly = useReadOnly(client);
  const connected = useConnected(client);
  const needsYou = useNeedsYou(client);
  const waiting = countOf(needsYou.state);
  return (
    <div className="min-h-svh">
      {readOnly ? <ReadOnlyBanner /> : null}
      <header className="border-b">
        <nav
          aria-label={t`Console`}
          className="mx-auto flex max-w-5xl flex-wrap gap-x-4 gap-y-2 px-6 py-3"
        >
          <MenuLink route="/" current={screen === undefined}>
            <Trans>Overview</Trans>
          </MenuLink>
          {SCREENS.filter((candidate) => candidate.inMenu).map((candidate) => (
            <MenuLink key={candidate.route} route={candidate.route} current={candidate === screen}>
              {i18n._(candidate.name)}
              {candidate.route === NEEDS_YOU && waiting > 0 ? (
                <span
                  className="ms-1.5 rounded-full bg-primary px-1.5 text-sm text-primary-foreground"
                  translate="no"
                >
                  {formatCount(waiting)}
                </span>
              ) : null}
            </MenuLink>
          ))}
          {client === undefined || onSignOut === undefined ? null : (
            <button
              type="button"
              disabled={!connected}
              onClick={onSignOut}
              className="ms-auto rounded-sm underline-offset-4 hover:underline focus-visible:ring-3 focus-visible:ring-ring focus-visible:outline-none disabled:text-muted-foreground disabled:no-underline"
            >
              <Trans>Sign out</Trans>
            </button>
          )}
        </nav>
      </header>
      {client === undefined ? null : <NeedsYouAnnouncement state={needsYou.state} />}
      {at === undefined ? (
        <main className="mx-auto flex max-w-2xl flex-col justify-center gap-3 px-6 py-16">
          <Overview client={client} product={build.product} />
        </main>
      ) : (
        <main className="mx-auto flex max-w-5xl flex-col gap-3 px-6 py-8">
          {client === undefined ? (
            <output className="block">
              <NotSignedIn product={build.product} />
            </output>
          ) : (
            at.screen.content(
              { client, address, needsYou, readOnly, offline, runViews: build.runViews },
              at.params,
            )
          )}
        </main>
      )}
    </div>
  );
}
