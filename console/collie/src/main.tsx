// Collie's Console build entry (ADR 0087): signs in through the link's
// token when the address carries one, then connects the Console's Client
// over the WebSocket on its own origin and renders the shell, with the
// Console's messages and design system.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import {
  documentVisibility,
  httpBodyFetcher,
  protocolUrl,
  webSocketTransport,
} from "./client/browser.ts";
import { createConsoleClient } from "./client/client.ts";
import { ConsoleI18n } from "./i18n.tsx";
import { Shell } from "./shell.tsx";
import { signIn } from "./sign-in.ts";
import "./styles.css";

const root = document.getElementById("root");
if (root === null) {
  throw new Error("invariant: index.html has a #root element");
}
const origin = window.location.origin;
const signedIn = await signIn({
  location: window.location,
  history: window.history,
  fetch: window.fetch.bind(window),
});
const client = signedIn
  ? createConsoleClient({
      transport: webSocketTransport(protocolUrl(origin)),
      bodyFetcher: httpBodyFetcher(origin),
      visibility: documentVisibility(document),
      product: "collie",
    })
  : undefined;
createRoot(root).render(
  <StrictMode>
    <ConsoleI18n>
      <Shell client={client} route={window.location.pathname} />
    </ConsoleI18n>
  </StrictMode>,
);
