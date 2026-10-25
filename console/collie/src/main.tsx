// Collie's Console build entry (ADR 0087): signs in through the link's
// token when the address carries one, then connects the Console's Client
// over the WebSocket on its own origin, or over Server-Sent Events plus
// `POST` where WebSockets are refused, opens the sign-in's offline cache
// (ADR 0031), and renders the shell, with the Console's messages and design
// system, over the window's address, following the Console's own links
// without loading the page again. A page signed out, or under
// `console.offline_cache` off, wipes the offline cache, and so does Sign
// out. The document takes the language and direction of the catalog it
// reads.
import { StrictMode, useSyncExternalStore, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { windowAddress } from "./address.ts";
import {
  documentVisibility,
  eventStreamTransport,
  httpBodyFetcher,
  protocolUrl,
  webSocketTransport,
  withFallback,
} from "./client/browser.ts";
import { createConsoleClient } from "./client/client.ts";
import { applyLocale, ConsoleI18n, i18n } from "./i18n.tsx";
import { followLinks } from "./navigation.ts";
import {
  daemonOfflineKeys,
  openOfflineCache,
  wipeOfflineCache,
  type OfflineCache,
} from "./offline/offline.ts";
import { Shell } from "./shell.tsx";
import { signIn, signOut } from "./sign-in.ts";
import "./styles.css";

const root = document.getElementById("root");
if (root === null) {
  throw new Error("invariant: index.html has a #root element");
}
applyLocale(document.documentElement, i18n.locale);
const origin = window.location.origin;
const fetcher = window.fetch.bind(window);
const signedIn = await signIn({
  location: window.location,
  history: window.history,
  fetch: fetcher,
});
let offline: OfflineCache | undefined;
if (signedIn?.offlineCache === true) {
  offline = await openOfflineCache(
    window.indexedDB,
    daemonOfflineKeys(origin, signedIn.csrfToken, fetcher),
  );
} else {
  await wipeOfflineCache(window.indexedDB);
}
const onSignOut =
  signedIn === undefined
    ? undefined
    : (): void => {
        void (async () => {
          await wipeOfflineCache(window.indexedDB);
          await signOut({ location: window.location, fetch: fetcher }, signedIn);
          window.location.assign("/");
        })();
      };
const client =
  signedIn === undefined
    ? undefined
    : createConsoleClient({
        transport: withFallback(
          webSocketTransport(protocolUrl(origin)),
          eventStreamTransport(origin, signedIn.csrfToken, fetcher),
        ),
        bodyFetcher: httpBodyFetcher(origin, fetcher),
        visibility: documentVisibility(document),
        product: "collie",
      });
const address = windowAddress(window.location, window.history);

/** Hears each move between the Console's screens. */
const followed = (changed: () => void): (() => void) => followLinks(window, changed);

/** The shell at the window's route, which moves as the Console follows its own links. */
function Page(): ReactNode {
  const route = useSyncExternalStore(followed, () => window.location.pathname);
  return (
    <Shell
      client={client}
      route={route}
      address={address}
      offline={offline}
      onSignOut={onSignOut}
    />
  );
}

createRoot(root).render(
  <StrictMode>
    <ConsoleI18n>
      <Page />
    </ConsoleI18n>
  </StrictMode>,
);
