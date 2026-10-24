// Collie's Console build entry (ADR 0087): signs in through the link's
// token when the address carries one, then connects the Console's Client
// over the WebSocket on its own origin, or over Server-Sent Events plus
// `POST` where WebSockets are refused, and renders the shell, with the
// Console's messages and design system, over the window's address. The
// document takes the language and direction of the catalog it reads.
import { StrictMode } from "react";
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
import { Shell } from "./shell.tsx";
import { signIn } from "./sign-in.ts";
import "./styles.css";

const root = document.getElementById("root");
if (root === null) {
  throw new Error("invariant: index.html has a #root element");
}
applyLocale(document.documentElement, i18n.locale);
const origin = window.location.origin;
const signedIn = await signIn({
  location: window.location,
  history: window.history,
  fetch: window.fetch.bind(window),
});
const client =
  signedIn === undefined
    ? undefined
    : createConsoleClient({
        transport: withFallback(
          webSocketTransport(protocolUrl(origin)),
          eventStreamTransport(origin, signedIn.csrfToken, window.fetch.bind(window)),
        ),
        bodyFetcher: httpBodyFetcher(origin, window.fetch.bind(window)),
        visibility: documentVisibility(document),
        product: "collie",
      });
createRoot(root).render(
  <StrictMode>
    <ConsoleI18n>
      <Shell
        client={client}
        route={window.location.pathname}
        address={windowAddress(window.location, window.history)}
      />
    </ConsoleI18n>
  </StrictMode>,
);
