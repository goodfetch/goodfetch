// Collie's Console build entry (ADR 0087): the shell, with the Console's
// messages and design system.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { ConsoleI18n } from "./i18n.tsx";
import { Shell } from "./shell.tsx";
import "./styles.css";

const root = document.getElementById("root");
if (root === null) {
  throw new Error("invariant: index.html has a #root element");
}
createRoot(root).render(
  <StrictMode>
    <ConsoleI18n>
      <Shell />
    </ConsoleI18n>
  </StrictMode>,
);
